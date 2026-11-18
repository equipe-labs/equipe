//! The kinds of work the person stopped ([`equipe_protocol::requests::Stop`]):
//! the rules, with no runtime in them — which terms name a chain of tickets
//! ([`terms_of`]), whether a new ticket is of a stopped kind ([`stopping`]),
//! what a member is told when it is turned away ([`refusal`]) and at the
//! start of its turns ([`context`]) — and where stops are kept ([`Store`]).
//!
//! The runtime's side — cancelling the chain, turning tickets away — is
//! `runtime::stops`. A stop holds until the person lifts it: cancelling
//! tickets alone didn't, as members filed the same kind of work again.

use std::collections::BTreeMap;

use equipe_protocol::requests::{RequestId, Stop, StopId};
use equipe_protocol::TicketId;
use rusqlite::{Connection, OptionalExtension as _, params};

/// The board tool a member stops a kind of work with, when the person said so.
pub const STOP_WORK: &str = "stop_work";

/// At most this many terms are taken from a chain's titles.
const TERMS_TAKEN: usize = 2;

/// Han, Hangul and Katakana: written without spaces between words, matched
/// two characters at a time.
fn dense(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7A3 | 0x30A0..=0x30FF)
}

/// The terms of a title: each word of four letters or more, lowercase, and
/// each pair of dense characters side by side.
fn terms(title: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in title.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let chars: Vec<char> = word.chars().collect();
        for pair in chars.windows(2).filter(|p| p.iter().all(|c| dense(*c))) {
            out.push(pair.iter().collect());
        }
        if chars.len() >= 4 && chars.iter().all(|c| c.is_alphabetic() && !dense(*c)) {
            out.push(word.to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// What names a chain of tickets, from their titles: the terms most of them
/// share (in at least half, two titles at least when there are two), the
/// most shared and longest first, as one term — a ticket is of the chain
/// when its title has every one. One title alone: its two longest words.
pub fn terms_of(titles: &[&str]) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for title in titles {
        for term in terms(title) {
            *seen.entry(term).or_default() += 1;
        }
    }
    let least = titles.len().div_ceil(2).max(titles.len().min(2));
    let mut shared: Vec<(usize, String)> = seen.into_iter().filter(|(_, n)| *n >= least).map(|(t, n)| (n, t)).collect();
    shared.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.chars().count().cmp(&a.1.chars().count())).then(a.1.cmp(&b.1)));
    let words: Vec<String> = shared.into_iter().take(TERMS_TAKEN).map(|(_, t)| t).collect();
    if words.is_empty() { Vec::new() } else { vec![words.join(" ")] }
}

/// Whether `title` has `word`: a dense pair inside it, or a word of it
/// starting as `word` does (its first six letters: `gallery` is in
/// `galleries`).
fn has_word(title: &str, word: &str) -> bool {
    let title = title.to_lowercase();
    let word = word.to_lowercase();
    if word.chars().any(dense) {
        return title.contains(&word);
    }
    let stem: String = word.chars().take(6).collect();
    !stem.is_empty() && title.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&stem))
}

/// Whether a ticket titled `title`, with `body`, is named by `stop`'s terms
/// (every word of one of them in the title) or paths (one in either).
pub fn named_by(stop: &Stop, title: &str, body: &str) -> bool {
    let term = stop.terms.iter().any(|t| {
        let words: Vec<&str> = t.split_whitespace().collect();
        !words.is_empty() && words.iter().all(|w| has_word(title, w))
    });
    term || stop.paths.iter().map(|p| p.trim()).filter(|p| p.len() >= 3).any(|p| title.contains(p) || body.contains(p))
}

/// The stop a new ticket would break, if any: one it comes of (a ticket of
/// `lineage` — what it was made while working, what it follows up — is in
/// the stopped chain), serves (`request`), or is named by.
pub fn stopping<'a>(stops: &'a [Stop], title: &str, body: &str, lineage: &[TicketId], request: Option<RequestId>) -> Option<&'a Stop> {
    stops.iter().find(|s| lineage.iter().any(|t| s.tickets.contains(t)) || (request.is_some() && s.request == request) || named_by(s, title, body))
}

/// When a stop began, as a member is told: its date.
fn since(stop: &Stop) -> String {
    equipe_providers::platform::local_time(stop.since).map(|t| format!("{}-{:02}-{:02}", t.year, t.month, t.day)).unwrap_or_default()
}

/// What a member is told when its ticket is turned away (English, as every
/// word to a model).
pub fn refusal(stop: &Stop) -> String {
    format!(
        "Not put on the board: the person stopped this kind of work — “{}” (stop S-{}, since {}). Don't file it, or work \
around it under another name. If you think it is needed now, ask the person first, in a line, and go on with something \
else; only they lift the stop.",
        stop.words,
        stop.id,
        since(stop)
    )
}

/// What every member's turn starts with while stops are in force. `None` without any.
pub fn context(stops: &[Stop]) -> Option<String> {
    if stops.is_empty() {
        return None;
    }
    let lines: Vec<String> = stops
        .iter()
        .map(|s| {
            let mut what = Vec::new();
            if !s.terms.is_empty() {
                what.push(format!("titles with {}", s.terms.iter().map(|t| format!("“{t}”")).collect::<Vec<_>>().join(" or ")));
            }
            if !s.paths.is_empty() {
                what.push(format!("work on {}", s.paths.join(", ")));
            }
            if let Some(r) = s.request {
                what.push(format!("request R-{r}"));
            }
            if !s.tickets.is_empty() {
                what.push(format!("follow-ups of T-{}", s.tickets.iter().map(u32::to_string).collect::<Vec<_>>().join(", T-")));
            }
            format!("- S-{} “{}” (since {}): {}", s.id, s.words, since(s), what.join("; "))
        })
        .collect();
    Some(format!(
        "[equipe: the person stopped these kinds of work. Don't put tickets of them on the board — equipe turns them away — \
nor polish or follow up on them under another name; if one seems needed, ask the person first:\n{}]",
        lines.join("\n")
    ))
}

// ── where stops are kept ──

pub fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS stops (
           id INTEGER PRIMARY KEY,
           words TEXT NOT NULL,
           terms TEXT NOT NULL DEFAULT '',
           paths TEXT NOT NULL DEFAULT '',
           request INTEGER,
           tickets TEXT NOT NULL DEFAULT '',
           by TEXT NOT NULL,
           since INTEGER NOT NULL,
           cancelled TEXT NOT NULL DEFAULT '',
           refused INTEGER NOT NULL DEFAULT 0,
           lifted_at INTEGER
         );",
    )
}

fn ids(text: &str) -> Vec<TicketId> {
    text.split(',').filter_map(|n| n.trim().parse().ok()).collect()
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

fn joined(ids: &[TicketId]) -> String {
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// The squad's stops in its journal.
pub struct Store<'a>(pub(crate) &'a Connection);

impl Store<'_> {
    /// Keep a new stop, in force from `stop.since`; its number.
    pub fn create(&self, stop: &Stop) -> rusqlite::Result<StopId> {
        self.0.execute(
            "INSERT INTO stops (words, terms, paths, request, tickets, by, since, cancelled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                stop.words,
                stop.terms.join("\n"),
                stop.paths.join("\n"),
                stop.request.map(|r| r as i64),
                joined(&stop.tickets),
                stop.by,
                stop.since as i64,
                joined(&stop.cancelled)
            ],
        )?;
        Ok(self.0.last_insert_rowid() as StopId)
    }

    /// The tickets `stop` cancelled.
    pub fn set_cancelled(&self, stop: StopId, cancelled: &[TicketId]) -> rusqlite::Result<()> {
        self.0.execute("UPDATE stops SET cancelled = ?2 WHERE id = ?1", params![stop as i64, joined(cancelled)])?;
        Ok(())
    }

    /// One more ticket turned away by `stop`.
    pub fn refused(&self, stop: StopId) -> rusqlite::Result<()> {
        self.0.execute("UPDATE stops SET refused = refused + 1 WHERE id = ?1", [stop as i64])?;
        Ok(())
    }

    /// Lifted at `at`: no longer in force. `false` when it wasn't.
    pub fn lift(&self, stop: StopId, at: u64) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE stops SET lifted_at = ?2 WHERE id = ?1 AND lifted_at IS NULL", params![stop as i64, at as i64])? > 0)
    }

    /// Those in force, the oldest first.
    pub fn in_force(&self) -> Vec<Stop> {
        let Ok(mut stmt) = self.0.prepare("SELECT id, words, terms, paths, request, tickets, by, since, cancelled, refused FROM stops WHERE lifted_at IS NULL ORDER BY id") else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(Stop {
                id: r.get::<_, i64>(0)? as StopId,
                words: r.get(1)?,
                terms: lines(&r.get::<_, String>(2)?),
                paths: lines(&r.get::<_, String>(3)?),
                request: r.get::<_, Option<i64>>(4)?.map(|n| n as RequestId),
                tickets: ids(&r.get::<_, String>(5)?),
                by: r.get(6)?,
                since: r.get::<_, i64>(7)?.max(0) as u64,
                cancelled: ids(&r.get::<_, String>(8)?),
                refused: r.get::<_, i64>(9)?.max(0) as u32,
            })
        })
        .and_then(|rows| rows.collect())
        .unwrap_or_default()
    }

    /// The design follow-ups that came of `ticket` (its own, and theirs).
    pub fn follow_ups_of(&self, ticket: TicketId) -> Vec<TicketId> {
        let Ok(mut stmt) = self.0.prepare("SELECT ticket FROM design_follow_ups WHERE parent = ?1 OR root = ?1 ORDER BY ticket") else { return Vec::new() };
        stmt.query_map([ticket], |r| r.get(0)).and_then(|rows| rows.collect()).unwrap_or_default()
    }

    pub fn get(&self, stop: StopId) -> Option<Stop> {
        let found: Option<i64> = self.0.query_row("SELECT id FROM stops WHERE id = ?1", [stop as i64], |r| r.get(0)).optional().ok().flatten();
        found.and_then(|_| self.in_force().into_iter().find(|s| s.id == stop))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(terms: &[&str], paths: &[&str], tickets: &[TicketId], request: Option<RequestId>) -> Stop {
        Stop {
            id: 1,
            words: "stop polishing the component gallery".into(),
            terms: terms.iter().map(|t| t.to_string()).collect(),
            paths: paths.iter().map(|t| t.to_string()).collect(),
            request,
            tickets: tickets.to_vec(),
            by: "you".into(),
            since: 0,
            cancelled: vec![],
            refused: 0,
        }
    }

    #[test]
    fn a_chains_titles_name_it_by_what_most_of_them_share() {
        let titles = ["Polish the component gallery spacing", "Gallery: fix card radius in the component gallery", "Component gallery dark mode"];
        assert_eq!(terms_of(&titles), ["component gallery"]);
        assert_eq!(terms_of(&["갤러리 카드 간격 다듬기", "갤러리 다크 모드"]), ["갤러 러리"]);
        assert_eq!(terms_of(&["Spec", "Fix"]), Vec::<String>::new(), "nothing shared, nothing named");
        assert_eq!(terms_of(&["Polish the component gallery"]), ["component gallery"]);
    }

    #[test]
    fn a_new_ticket_is_of_a_stopped_kind_by_lineage_request_terms_or_paths() {
        let stops = [stop(&["component gallery"], &["src/gallery/"], &[40, 41], Some(7))];
        let of = |title: &str, body: &str, lineage: &[TicketId], request| stopping(&stops, title, body, lineage, request).is_some();
        assert!(of("Tighten the Component Galleries grid", "", &[], None), "every word of a term, by its stem");
        assert!(!of("Gallery of customer logos", "", &[], None), "one word of it isn't enough");
        assert!(of("Fix the card radius", "In src/gallery/Card.tsx", &[], None), "a path it names");
        assert!(of("Fix the card radius", "", &[41], None), "made while working the stopped chain");
        assert!(of("Anything", "", &[], Some(7)), "serving the stopped request");
        assert!(!of("Build the checkout", "", &[3], Some(8)));
        let dense = [stop(&["갤러 러리"], &[], &[], None)];
        assert!(stopping(&dense, "컴포넌트 갤러리 여백 손보기", "", &[], None).is_some());
        assert!(stopping(&dense, "결제 화면", "", &[], None).is_none());
    }

    #[test]
    fn a_member_is_told_what_is_stopped_and_to_ask_first() {
        let s = stop(&["component gallery"], &[], &[40], None);
        let text = refusal(&s);
        assert!(text.contains("“stop polishing the component gallery”") && text.contains("S-1") && text.contains("ask the person first"), "{text}");
        let ctx = context(std::slice::from_ref(&s)).unwrap();
        assert!(ctx.contains("titles with “component gallery”") && ctx.contains("follow-ups of T-40"), "{ctx}");
        assert_eq!(context(&[]), None);
    }

    #[test]
    fn stops_are_kept_until_lifted() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        let store = Store(&conn);
        let id = store.create(&stop(&["component gallery"], &["src/gallery/"], &[40, 41], Some(7))).unwrap();
        store.set_cancelled(id, &[41]).unwrap();
        store.refused(id).unwrap();
        let kept = store.in_force();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].tickets.clone(), kept[0].cancelled.clone(), kept[0].refused, kept[0].request), (vec![40, 41], vec![41], 1, Some(7)));
        assert!(store.lift(id, 5).unwrap());
        assert!(!store.lift(id, 6).unwrap(), "lifted once");
        assert!(store.in_force().is_empty());
        assert!(store.get(id).is_none());
    }
}
