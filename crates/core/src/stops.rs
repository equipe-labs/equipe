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

/// The platform's tools that put work on the live site: refused while all work is paused.
pub const GOES_LIVE: [&str; 2] = ["deploy_site", "promote_version"];

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
    stops.iter().find(|s| !s.all && (lineage.iter().any(|t| s.tickets.contains(t)) || (request.is_some() && s.request == request) || named_by(s, title, body)))
}

/// The pause of all work in force, if any (the newest).
pub fn pause(stops: &[Stop]) -> Option<&Stop> {
    stops.iter().rev().find(|s| s.all)
}

/// Whether a ticket may go on while `pause` holds: only work for what the
/// person asked after the pause began (`asked_at` of the request it serves).
pub fn goes_on_in(pause: &Stop, asked_at: Option<u64>) -> bool {
    asked_at.is_some_and(|at| at > pause.since)
}

/// Whether `text` (the person's words) stops the *other* work — "stop the
/// rest", "pause everything else" — so that what they talk about goes on.
/// Read as one thing to stop, the PM cancelled the very drafts the person
/// wanted finished (inhwadoen, 2026-11-28). The words are each language's
/// (`match.stop_others`).
pub fn stops_the_others(text: &str) -> bool {
    let t = text.to_lowercase();
    crate::i18n::every("match.stop_others").iter().any(|w| t.contains(&w.to_lowercase()))
}

/// Whether `text` (the person's words) says "pause everything": all of the
/// team's work, not one kind of it.
pub fn pauses_all(text: &str) -> bool {
    let t = text.to_lowercase();
    let whole = ["everything", "all work", "all the work", "all of it", "anything new", "any work", "the whole team", "all tasks"];
    let verb = ["pause", "stop", "hold", "wait", "freeze", "halt"];
    let keep_going = ["don't stop", "dont stop", "do not stop", "never stop", "don't pause", "do not pause", "keep going"];
    verb.iter().any(|v| t.contains(v)) && whole.iter().any(|w| t.contains(w)) && !keep_going.iter().any(|k| t.contains(k))
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
            if s.all {
                return format!("- S-{} “{}” (since {}): ALL work is paused — start nothing, land nothing, deploy nothing until the person says go; work for what they ask after it goes on", s.id, s.words, since(s));
            }
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
           lifted_at INTEGER,
           all_work INTEGER NOT NULL DEFAULT 0,
           heads TEXT NOT NULL DEFAULT ''
         );",
    )?;
    // Journals from before pauses of all work and stopped commits.
    let _ = conn.execute("ALTER TABLE stops ADD COLUMN all_work INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE stops ADD COLUMN heads TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE stops ADD COLUMN keep TEXT NOT NULL DEFAULT ''", []);
    Ok(())
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
            "INSERT INTO stops (words, terms, paths, request, tickets, by, since, cancelled, all_work, keep) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                stop.words,
                stop.terms.join("\n"),
                stop.paths.join("\n"),
                stop.request.map(|r| r as i64),
                joined(&stop.tickets),
                stop.by,
                stop.since as i64,
                joined(&stop.cancelled),
                stop.all as i64,
                joined(&stop.keep)
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
        let Ok(mut stmt) = self.0.prepare("SELECT id, words, terms, paths, request, tickets, by, since, cancelled, refused, all_work, keep FROM stops WHERE lifted_at IS NULL ORDER BY id") else {
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
                all: r.get::<_, i64>(10)? != 0,
                keep: ids(&r.get::<_, String>(11)?),
            })
        })
        .and_then(|rows| rows.collect())
        .unwrap_or_default()
    }

    /// The commits of `stop`'s chain that weren't on main when it began
    /// (`ticket`, branch tip): kept so none lands later inside other work.
    pub fn set_heads(&self, stop: StopId, heads: &[(TicketId, String)]) -> rusqlite::Result<()> {
        let text = heads.iter().map(|(t, c)| format!("{t}:{c}")).collect::<Vec<_>>().join(",");
        self.0.execute("UPDATE stops SET heads = ?2 WHERE id = ?1", params![stop as i64, text])?;
        Ok(())
    }

    /// The stopped commits of the stops in force: (stop, ticket, commit).
    pub fn heads_in_force(&self) -> Vec<(StopId, TicketId, String)> {
        let Ok(mut stmt) = self.0.prepare("SELECT id, heads FROM stops WHERE lifted_at IS NULL AND heads <> '' ORDER BY id") else { return Vec::new() };
        let rows: Vec<(i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).and_then(|rows| rows.collect()).unwrap_or_default();
        rows.into_iter()
            .flat_map(|(id, text)| {
                text.split(',')
                    .filter_map(|pair| pair.split_once(':'))
                    .filter_map(|(t, c)| Some((id as StopId, t.trim().parse().ok()?, c.trim().to_owned())))
                    .filter(|(_, _, c): &(StopId, TicketId, String)| c.len() >= 7 && c.chars().all(|ch| ch.is_ascii_hexdigit()))
                    .collect::<Vec<_>>()
            })
            .collect()
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
            all: false,
            keep: Vec::new(),
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
    fn a_pause_of_all_work_is_read_from_the_persons_words_and_holds_only_what_came_before() {
        // Golden Boot's words, and others like them.
        for said in ["Please pause all work right now: stop T-29, T-30 and T-31 and don't start anything new.", "Stop everything for now", "hold all the work until tomorrow", "Pause the whole team please"] {
            assert!(pauses_all(said), "{said}");
        }
        for said in ["stop polishing the gallery", "pause the newsletter", "everything looks great", "wait for my go-ahead on the logo", "Don't stop, everything is great"] {
            assert!(!pauses_all(said), "{said}");
        }
        // "Stop the other work": what they talk about goes on (inhwadoen, 2026-11-28, the drafts were cancelled instead).
        for said in ["시안 결정될 때까지 다른 작업은 멈춰줘", "Stop the other work until I pick a draft", "Pause everything else for now", "나머지 작업은 잠깐 멈춰"] {
            assert!(stops_the_others(said), "{said}");
        }
        for said in ["stop polishing the gallery", "갤러리 다듬는 건 그만해", "pause everything"] {
            assert!(!stops_the_others(said), "{said}");
        }
        let pause = Stop { all: true, since: 1_000, ..stop(&[], &[], &[], None) };
        assert!(!goes_on_in(&pause, None) && !goes_on_in(&pause, Some(900)) && !goes_on_in(&pause, Some(1_000)), "work from before waits");
        assert!(goes_on_in(&pause, Some(1_001)), "what the person asks after goes on");
        // A pause names no kind of work: it turns no ticket away by its title, it holds them all.
        let stops = [pause.clone(), stop(&["component gallery"], &[], &[], None)];
        assert_eq!(super::pause(&stops).map(|s| s.since), Some(1_000));
        assert!(stopping(&stops, "Main is red: Lighthouse 91", "", &[], None).is_none());
        assert!(stopping(&stops, "Component gallery spacing", "", &[], None).is_some());
        assert!(context(&[pause]).unwrap().contains("ALL work is paused"));
    }

    #[test]
    fn a_pause_and_the_stopped_commits_are_kept_with_the_stop() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn).unwrap();
        create_table(&conn).unwrap(); // again: the columns are there already
        let store = Store(&conn);
        let id = store.create(&Stop { all: true, keep: vec![16, 17], ..stop(&[], &[], &[29], None) }).unwrap();
        assert_eq!(store.get(id).map(|s| s.keep), Some(vec![16, 17]), "what goes on is kept with the pause");
        store.set_heads(id, &[(29, "a1b2c3d4e5f6".into()), (30, "not-a-commit".into())]).unwrap();
        assert!(store.in_force()[0].all);
        assert_eq!(store.heads_in_force(), [(id, 29, "a1b2c3d4e5f6".to_owned())]);
        store.lift(id, 5).unwrap();
        assert!(store.heads_in_force().is_empty(), "a lifted stop holds nothing back");
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
