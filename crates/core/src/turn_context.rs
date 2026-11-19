//! What equipe puts in front of a message to a member — where each project
//! stands, what waits on the person, its open cards, the person's requests,
//! the stops in force — said once per conversation, and after that only what
//! changed.
//!
//! Every block a message carries stays in the conversation, and every later
//! model call reads the whole conversation again. A PM woken every half hour
//! with the same 12k characters of overview, waiting list and cards in front
//! of each message filled its 200k window in a handful of turns. So each
//! block is remembered per conversation ([`Told`]): the same block again is
//! left out (or, where the count is what matters, reduced to a line saying it
//! still stands), a changed one is sent as its changed lines and those that no
//! longer hold, and one that is gone is said to be gone. A fresh conversation
//! — a new engine session, a compaction, a daemon restart — hears every block
//! whole again.
//!
//! [`compose`] is pure: blocks and memory in, the prompt and what each block
//! weighed out, so the rules are tested without an engine. What each block
//! weighed is kept per PM turn (`equipe usage --pm-context`).

use std::collections::HashMap;

use equipe_protocol::{Ticket, TicketStatus};

/// The longest a line that no longer holds is quoted.
const GONE_QUOTE: usize = 100;
/// The most lines that no longer hold that are quoted; the rest are counted.
const GONE_SHOWN: usize = 10;

/// One block in front of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// What it is, as it is remembered and measured: `overview`, `waiting`, …
    pub key: &'static str,
    /// Its text: its first line says what it is, each further line one item.
    /// `None`: nothing to say now (no cards open, nothing waits).
    pub body: Option<String>,
    /// Said again every time, never remembered: a line that frames the
    /// message (which project it is about, that it is read on a phone).
    pub every_time: bool,
    /// The line sent in its place when it is as last said — where what it
    /// counts matters at every turn (how many things wait on the person).
    /// `None`: left out then.
    pub still: Option<String>,
    /// What is said when it was said before and now has nothing — none of
    /// it holds any more. `None`: nothing (absent doesn't mean empty).
    pub gone: Option<&'static str>,
}

impl Part {
    /// A block remembered per conversation.
    pub fn told(key: &'static str, body: Option<String>) -> Self {
        Part { key, body, every_time: false, still: None, gone: None }
    }

    /// A framing line said with every message.
    pub fn every_time(key: &'static str, body: String) -> Self {
        Part { key, body: Some(body), every_time: true, still: None, gone: None }
    }

    pub fn still(mut self, line: impl Into<String>) -> Self {
        self.still = Some(line.into());
        self
    }

    pub fn gone(mut self, line: &'static str) -> Self {
        self.gone = Some(line);
        self
    }
}

/// What one block weighed in a turn: in full, and as sent (characters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    pub key: &'static str,
    pub full: usize,
    pub sent: usize,
}

/// What a conversation has been told of each block, as last sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Told {
    /// The engine session it was told in; `None` before the first turn of a
    /// fresh one names it (or on an engine that names none).
    conversation: Option<String>,
    parts: HashMap<&'static str, Vec<String>>,
}

impl Told {
    /// As a turn begins in the conversation `handle` names: `fresh` when it
    /// starts with nothing of before in it (no engine session to resume, no
    /// history carried), and every block is said whole again. A conversation
    /// told before under another name — a thread switched — is not this one.
    pub fn begin(&mut self, handle: Option<&str>, fresh: bool) {
        if fresh {
            self.forget();
            return;
        }
        match (&self.conversation, handle) {
            // Its first turn named it: what was told, was told there.
            (None, Some(h)) => self.conversation = Some(h.to_owned()),
            (Some(was), Some(h)) if was != h => {
                self.parts.clear();
                self.conversation = Some(h.to_owned());
            }
            _ => {}
        }
    }

    /// Nothing of before stands: compacted, or started over.
    pub fn forget(&mut self) {
        self.parts.clear();
        self.conversation = None;
    }
}

/// The text of `parts` in front of `body`, each block whole: for a prompt
/// that waits to be sent (its conversation may have moved on by then).
pub fn full(parts: &[Part], body: &str) -> String {
    let mut out: Vec<String> = parts.iter().filter_map(|p| p.body.as_deref().map(bracket)).collect();
    out.push(body.to_owned());
    out.retain(|s| !s.is_empty());
    out.join("\n\n")
}

/// The prompt: `parts` in front of `body`, each as `told` makes it — whole
/// the first time in a conversation, then only what changed — and what each
/// weighed. `told` is brought up to what this prompt says.
pub fn compose(told: &mut Told, parts: &[Part], body: &str) -> (String, Vec<Measure>) {
    let mut out: Vec<String> = Vec::new();
    let mut measures = Vec::new();
    for part in parts {
        let full_text = part.body.as_deref().map(bracket).unwrap_or_default();
        let sent = match (&part.body, part.every_time) {
            (Some(_), true) => Some(full_text.clone()),
            (Some(body), false) => {
                let lines: Vec<String> = body.lines().map(str::to_owned).collect();
                let said = match told.parts.get(part.key) {
                    None => Some(full_text.clone()),
                    Some(before) if *before == lines => part.still.as_deref().map(bracket),
                    Some(before) => Some(changes(before, &lines).filter(|d| d.len() < full_text.len()).unwrap_or_else(|| full_text.clone())),
                };
                told.parts.insert(part.key, lines);
                said
            }
            (None, _) => match told.parts.remove(part.key) {
                Some(_) => part.gone.map(bracket),
                None => None,
            },
        };
        measures.push(Measure { key: part.key, full: full_text.len(), sent: sent.as_ref().map_or(0, String::len) });
        if let Some(text) = sent.filter(|s| !s.is_empty()) {
            out.push(text);
        }
    }
    if !body.is_empty() {
        out.push(body.to_owned());
    }
    (out.join("\n\n"), measures)
}

/// `[equipe: …]` around a block's text, unless it carries its own brackets.
fn bracket(text: &str) -> String {
    if text.starts_with('[') { text.to_owned() } else { format!("[equipe: {text}]") }
}

/// A block's text without the `[equipe: …]` around it, for one built with it.
pub fn inner(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("[equipe: ").unwrap_or(text);
    text.strip_suffix(']').unwrap_or(text).to_owned()
}

/// What changed from `before` to `now`: the first line (what the block is)
/// when anything did, the lines new or changed, and those that no longer
/// hold. `None` when nothing did.
fn changes(before: &[String], now: &[String]) -> Option<String> {
    let head = now.first().cloned().unwrap_or_default();
    let new: Vec<&String> = now.iter().skip(1).filter(|l| !l.trim().is_empty() && !before.contains(l)).collect();
    let gone: Vec<&String> = before.iter().skip(1).filter(|l| !l.trim().is_empty() && !now.contains(l)).collect();
    if new.is_empty() && gone.is_empty() && before.first() == now.first() {
        return None;
    }
    let mut out = format!("[equipe: what changed since you were told this earlier in this conversation — {head}");
    for line in &new {
        out.push('\n');
        out.push_str(line);
    }
    if !gone.is_empty() {
        let quoted: Vec<String> = gone.iter().take(GONE_SHOWN).map(|l| clip(l.trim().trim_start_matches("- "), GONE_QUOTE)).collect();
        out.push_str(&format!("\nNo longer so: {}", quoted.join(" | ")));
        if gone.len() > GONE_SHOWN {
            out.push_str(&format!(" (and {} more)", gone.len() - GONE_SHOWN));
        }
    }
    out.push_str("\nThe rest stands as told.]");
    Some(out)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_owned() } else { format!("{}…", text.chars().take(max).collect::<String>()) }
}

/// The total characters `measures` sent.
pub fn sent(measures: &[Measure]) -> usize {
    measures.iter().map(|m| m.sent).sum()
}

/// How a message to the PM sent looking at the whole squad opens, every
/// time: what it is about (the conversation reads it back) and what to answer
/// for; where each project stands follows as a block of its own.
pub const WHOLE_SQUAD_FRAMING: &str = "[equipe: the person is looking at the whole squad — you oversee every project. Answer for the whole: priorities, \
what waits on the person, what goes next across projects. Work for one project goes to it as tickets for its owners, with that \
project — and you say where it went.]";

/// The most tickets of a kind a project's line in the PM's overview names.
pub const OVERVIEW_NAMED: usize = 5;
/// The longest a ticket's title is quoted there.
const OVERVIEW_TITLE: usize = 50;

/// What a project's line in the PM's overview says of its tickets, and the
/// lines under it: those in
/// review and those waiting on the person — a few named, the rest counted —
/// and how many are to do or in progress. What each waiting one needs is in
/// the list of what waits on the person, and the board has every note: not
/// here again.
pub fn project_tickets(here: &[&Ticket]) -> String {
    let named = |status: TicketStatus| -> Option<String> {
        let of: Vec<&&Ticket> = here.iter().filter(|t| t.status == status).collect();
        if of.is_empty() {
            return None;
        }
        let mut names: Vec<String> = of.iter().take(OVERVIEW_NAMED).map(|t| format!("T-{} ({})", t.id, clip(t.title.trim(), OVERVIEW_TITLE))).collect();
        if of.len() > OVERVIEW_NAMED {
            names.push(format!("and {} more", of.len() - OVERVIEW_NAMED));
        }
        Some(names.join(", "))
    };
    let mut out = String::new();
    let open = here.iter().filter(|t| matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress)).count();
    if open > 0 {
        out.push_str(&format!("; {open} to do or in progress"));
    }
    // Each list on a line of its own under the project's: one that changes is said again alone.
    if let Some(review) = named(TicketStatus::Review) {
        out.push_str(&format!("\n  waiting for the person's review: {review}"));
    }
    if let Some(waiting) = named(TicketStatus::Waiting) {
        out.push_str(&format!("\n  waiting on the person: {waiting}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(n: usize) -> Part {
        let mut body = format!("Waiting on the person ({n} in all), the most blocking first:");
        for i in 0..n {
            body.push_str(&format!("\n- T-{i} a thing to do — waiting 2d"));
        }
        Part::told("waiting", Some(body)).still(format!("{n} things wait on the person — as listed earlier")).gone("Nothing waits on the person now.")
    }

    #[test]
    fn a_block_is_said_whole_once_then_left_out_or_counted_while_it_stands() {
        let mut told = Told::default();
        told.begin(None, true);
        let parts = [Part::every_time("scope", "[equipe: the person is looking at the whole squad]".into()), Part::told("overview", Some("Where each stands now:\n- site: no one on it".into())), waiting(3)];
        let (first, m1) = compose(&mut told, &parts, "hello");
        assert!(first.starts_with("[equipe: the person is looking at the whole squad]") && first.contains("- site: no one on it") && first.contains("T-2 a thing") && first.ends_with("hello"), "{first}");
        assert_eq!(m1.iter().map(|m| m.full).collect::<Vec<_>>(), m1.iter().map(|m| m.sent).collect::<Vec<_>>());
        told.begin(Some("sess-1"), false);
        let (second, m2) = compose(&mut told, &parts, "again");
        assert!(second.starts_with("[equipe: the person is looking at the whole squad]"), "the framing every time: {second}");
        assert!(!second.contains("site: no one") && !second.contains("T-2 a thing"), "{second}");
        assert!(second.contains("3 things wait on the person"), "the count still said: {second}");
        assert!(sent(&m2) < sent(&m1) / 2, "{m2:?}");
    }

    #[test]
    fn a_changed_block_sends_what_changed_and_what_no_longer_holds() {
        let mut told = Told::default();
        compose(&mut told, &[waiting(10)], "");
        let mut now = waiting(10);
        now.body = Some(now.body.unwrap().replace("- T-1 a thing to do — waiting 2d", "- T-9 a new thing — waiting 1m"));
        let (said, _) = compose(&mut told, &[now], "");
        assert!(said.contains("- T-9 a new thing") && said.contains("No longer so: T-1 a thing to do"), "{said}");
        assert!(!said.contains("T-0 a thing"), "what stands isn't said again: {said}");
        // Gone altogether: said once, then nothing.
        let empty = Part { body: None, ..waiting(0) };
        let (said, _) = compose(&mut told, std::slice::from_ref(&empty), "");
        assert_eq!(said, "[equipe: Nothing waits on the person now.]");
        assert_eq!(compose(&mut told, &[empty], "").0, "");
    }

    #[test]
    fn a_fresh_conversation_another_one_or_a_compaction_hears_everything_again() {
        let parts = [waiting(2)];
        let mut told = Told::default();
        told.begin(None, true);
        compose(&mut told, &parts, "");
        told.begin(Some("a"), false);
        assert!(compose(&mut told, &parts, "").0.contains("as listed earlier"));
        // Another conversation (a thread switched): whole.
        told.begin(Some("b"), false);
        assert!(compose(&mut told, &parts, "").0.contains("T-1 a thing"));
        // Started over: whole.
        told.begin(None, true);
        assert!(compose(&mut told, &parts, "").0.contains("T-1 a thing"));
        told.forget();
        assert!(compose(&mut told, &parts, "").0.contains("T-1 a thing"));
        // An engine that names no session and carries its history: the same conversation.
        let mut told = Told::default();
        compose(&mut told, &parts, "");
        told.begin(None, false);
        assert!(!compose(&mut told, &parts, "").0.contains("T-1 a thing"));
    }

    #[test]
    fn a_block_with_its_own_brackets_keeps_them_and_full_says_everything() {
        let parts = [Part::told("requests", Some(inner("[equipe: what the person asked:\n- R-1 a page]")))];
        assert_eq!(full(&parts, "hi"), "[equipe: what the person asked:\n- R-1 a page]\n\nhi");
        let mut told = Told::default();
        assert_eq!(compose(&mut told, &parts, "hi").0, full(&parts, "hi"));
    }

    /// A typical busy squad: three projects, each with handed-in work, tickets
    /// put down for the person with long checklists, and plenty to do; cards
    /// and requests open. What a PM turn carries in front of the message is
    /// pinned: the first turn of a conversation, one where nothing changed,
    /// and one where one thing did.
    #[test]
    fn what_a_pm_turn_carries_for_a_typical_squad_stays_within_its_bounds() {
        let ticket = |id: u32, status: &str, project: &str, note: Option<String>| -> Ticket {
            let mut t: Ticket = serde_json::from_value(serde_json::json!({ "id": id, "title": format!("Make the {project} checkout keep the cart across sessions ({id})"), "body": "", "status": status, "depends_on": [], "created_by": "pm" })).unwrap();
            t.project = Some(std::path::PathBuf::from(project));
            t.note = note;
            t.updated_at = Some(1_000 + u64::from(id));
            t
        };
        let ask = "Open the provider's dashboard, create a deploy token scoped to this one repository only, and paste it into the card equipe shows; then confirm the preview deploy went green.";
        let mut tickets = Vec::new();
        let projects = ["site", "app", "api"];
        for (p, project) in projects.iter().enumerate() {
            let base = (p as u32 + 1) * 100;
            for i in 0..6 {
                tickets.push(ticket(base + i, "review", project, None));
            }
            for i in 10..22 {
                let asks: Vec<String> = (0..3).map(|n| format!("{ask} ({n})")).collect();
                tickets.push(ticket(base + i, "waiting", project, Some(equipe_protocol::asks::waiting_note(&asks, Some("for the deploy")))));
            }
            for i in 30..60 {
                tickets.push(ticket(base + i, "todo", project, None));
            }
        }
        let overview = |tickets: &[Ticket]| -> String {
            projects.iter().map(|p| {
                let here: Vec<&Ticket> = tickets.iter().filter(|t| t.project.as_deref() == Some(std::path::Path::new(p))).collect();
                format!("- {p}: no one on it now{}", project_tickets(&here))
            }).collect::<Vec<_>>().join("\n")
        };
        let now = 100_000;
        let parts = |tickets: &[Ticket]| -> Vec<Part> {
            let waiting = crate::importance::pm_note(&crate::importance::waiting_list(tickets, &[], true), crate::importance::PM_SHOWN, now);
            let cards = (0..6).map(|i| format!("#{i}: \"Which provider should the T-{i} preview deploy use, given the costs we discussed and the team's account?\"")).collect::<Vec<_>>().join("; ");
            let requests = (1..=4).map(|r| format!("- R-{r} \"Make checkout faster on phones\" — T-{r}01 in review, T-{r}10 waits on you")).collect::<Vec<_>>().join("\n");
            vec![
                Part::every_time("scope", WHOLE_SQUAD_FRAMING.into()),
                Part::told("overview", Some(format!("Where each project stands now:\n{}", overview(tickets)))).still("Where each project stands: as told earlier in this conversation, unchanged."),
                Part::told("cards", Some(format!("your decision cards still open for the person: {cards}. When their message settles one, withdraw it."))),
                Part::told("requests", Some(format!("what the person asked for in this conversation, and where each stands now:\n{requests}"))),
                Part::told("waiting", waiting.clone()).still(waiting.as_deref().map(|_| "36 things wait on the person — as listed earlier".to_owned()).unwrap_or_default()),
            ]
        };
        let mut told = Told::default();
        told.begin(None, true);
        let (_, first) = compose(&mut told, &parts(&tickets), "how is everything?");
        told.begin(Some("sess"), false);
        let (_, again) = compose(&mut told, &parts(&tickets), "and now?");
        // One waiting ticket done.
        let mut later = tickets.clone();
        if let Some(t) = later.iter_mut().find(|t| t.id == 110) {
            t.status = TicketStatus::Done;
        }
        let (_, changed) = compose(&mut told, &parts(&later), "and now?");
        // As every message carried it before: each block whole, each waiting ticket's needs in the overview.
        let needs_before: usize = tickets.iter().filter(|t| t.status == TicketStatus::Waiting).map(|t| t.note.as_deref().map_or(0, |n| crate::board::waiting_needs(n).join("; ").len() + t.title.len() + 40)).sum();
        let whole: usize = first.iter().map(|m| m.full).sum::<usize>() + needs_before;
        eprintln!("before, every turn: {whole} chars; now: first {} · unchanged {} · one change {}", sent(&first), sent(&again), sent(&changed));
        for m in first.iter().chain(&again).chain(&changed) {
            eprintln!("  {}: {} of {}", m.key, m.sent, m.full);
        }
        assert!(sent(&first) <= FIRST_TURN_MOST, "the first turn carries {} chars: {first:?}", sent(&first));
        assert!(sent(&again) <= UNCHANGED_TURN_MOST, "a turn where nothing changed carries {} chars: {again:?}", sent(&again));
        assert!(sent(&changed) <= CHANGED_TURN_MOST, "a turn where one thing changed carries {} chars: {changed:?}", sent(&changed));
        assert!(whole > 4 * sent(&first), "{whole}");
    }

    /// The most characters a typical squad's first PM turn carries in front of the message.
    const FIRST_TURN_MOST: usize = 7_000;
    /// … a turn after it where nothing changed: the framing and the counts.
    const UNCHANGED_TURN_MOST: usize = 600;
    /// … a turn where one waiting ticket was done.
    const CHANGED_TURN_MOST: usize = 2_500;
}
