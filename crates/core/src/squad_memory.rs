//! The squad's memory: `home/memory/`, what the whole squad knows about the
//! person and the work — facts about the person (preferences, schedule, the
//! services they use), decisions, recurring deadlines, terms, contacts.
//!
//! One fact per file, with a short frontmatter. equipe builds the index
//! (`memory/INDEX.md`) from the files themselves, so a file the person drops,
//! edits or deletes in Finder is in — or out of — the index every member is
//! given on its next turn. Members write it through equipe's `remember` and
//! `forget` tools, from their desk or a ticket alike; each change is saved in the
//! squad's folder like any other, by who made it, and can be taken back. What
//! the person wrote last wins: a member changes it only once they have agreed.

use std::path::{Path, PathBuf};

use crate::squad_home::{self, Change, PERSON};

/// The memory's folder inside the squad's home.
pub const DIR: &str = "memory";
/// The index equipe keeps, inside [`DIR`].
pub const INDEX: &str = "INDEX.md";
/// The index's path inside the home.
pub const INDEX_PATH: &str = "memory/INDEX.md";
/// What a fact can be about.
pub const KINDS: [&str; 6] = ["person", "decision", "deadline", "term", "contact", "other"];
/// Facts past which the PM is asked to propose tidying the memory: merging
/// what overlaps, dropping what's stale. A few screens of index.
pub const CROWDED: usize = 60;
/// Index lines a member is given; past these it is pointed at `INDEX.md`.
const SHOWN: usize = 150;

/// The memory's folder for the home at `folder`.
pub fn dir(folder: &Path) -> PathBuf {
    folder.join(DIR)
}

/// One remembered fact, as its file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// Its file, inside [`DIR`]: `tax-filing.md`.
    pub file: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    /// `YYYY-MM-DD`, when it was last written.
    pub updated: String,
    pub body: String,
}

/// Every fact in the memory, by file name. A file without frontmatter — the
/// person's own, say — is a fact too: named for its file, described by its
/// first line.
pub fn facts(folder: &Path) -> Vec<Fact> {
    let Ok(entries) = std::fs::read_dir(dir(folder)) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|f| f.ends_with(".md") && f != INDEX && !f.starts_with('.'))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(dir(folder).join(&file)).ok()?;
            Some(parse(&file, &text))
        })
        .collect()
}

fn parse(file: &str, text: &str) -> Fact {
    let stem = file.trim_end_matches(".md").to_owned();
    let mut fact = Fact { file: file.to_owned(), name: stem, description: String::new(), kind: "other".into(), updated: String::new(), body: String::new() };
    let rest = match text.strip_prefix("---\n").and_then(|t| t.split_once("\n---")) {
        Some((head, rest)) => {
            for line in head.lines() {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.trim().trim_matches('"').to_owned();
                match key.trim() {
                    "name" if !value.is_empty() => fact.name = value,
                    "description" => fact.description = value,
                    "kind" | "type" if !value.is_empty() => fact.kind = value,
                    "updated" => fact.updated = value,
                    _ => {}
                }
            }
            rest.strip_prefix('\n').unwrap_or(rest)
        }
        None => text,
    };
    fact.body = rest.trim().to_owned();
    if fact.description.is_empty() {
        let first = fact.body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or_default();
        fact.description = first.chars().take(120).collect();
    }
    fact
}

/// One fact as a line of the index.
fn line(fact: &Fact) -> String {
    let date = if fact.updated.is_empty() { String::new() } else { format!(", {}", fact.updated) };
    format!("- [{}]({}) ({}{date}) — {}", fact.name, fact.file, fact.kind, fact.description)
}

/// `INDEX.md` for these facts, headed in the person's language.
pub fn index_text(facts: &[Fact], language: Option<&str>) -> String {
    let words = crate::i18n::core(squad_home::named(language));
    let head = format!("# {}\n\n{}\n\n", words.get("squad_memory.index_title"), words.get("squad_memory.index_intro"));
    let lines: Vec<String> = facts.iter().map(line).collect();
    format!("{head}{}\n", lines.join("\n"))
}

/// Bring `INDEX.md` up to date with the files. Returns whether it changed.
/// With no fact and no index, nothing is written.
pub fn refresh_index(folder: &Path, facts: &[Fact], language: Option<&str>) -> bool {
    let path = dir(folder).join(INDEX);
    if facts.is_empty() && !path.exists() {
        return false;
    }
    let text = index_text(facts, language);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return false;
    }
    std::fs::create_dir_all(dir(folder)).is_ok() && std::fs::write(&path, text).is_ok()
}

/// The index as a member is given it each turn; `None` when nothing is remembered yet.
pub fn for_charter(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = facts.iter().take(SHOWN).map(line).collect();
    if facts.len() > SHOWN {
        lines.push(format!("- … and {} more: see {INDEX_PATH}", facts.len() - SHOWN));
    }
    Some(lines.join("\n"))
}

/// What a member asks to remember.
#[derive(Debug, Clone, Default)]
pub struct Remember<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub kind: &'a str,
    pub body: &'a str,
    /// The person agreed to change a fact they wrote last.
    pub confirmed: bool,
}

/// A fact's file name: its name, lowercased, with anything that isn't a letter
/// or a digit made a dash. Korean names stay Korean.
pub fn slug(name: &str) -> String {
    let dashed: String = name.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let slug = dashed.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    slug.chars().take(60).collect()
}

/// The file a name refers to: one whose frontmatter has that name, else its slug.
fn file_for(folder: &Path, name: &str) -> String {
    let slug = slug(name);
    facts(folder)
        .into_iter()
        .find(|f| f.name == name || f.file.trim_end_matches(".md") == slug)
        .map_or(format!("{slug}.md"), |f| f.file)
}

/// Write a fact down — a new one, or the one of that name — and save it in the
/// squad's folder, by `by`. Whatever the person changed in the memory since is
/// saved as theirs first. Returns the saved change and whether it was new.
pub fn remember(folder: &Path, by: &str, fact: &Remember<'_>, today: &str, language: Option<&str>) -> Result<(Change, bool), String> {
    let name = fact.name.trim();
    if slug(name).is_empty() {
        return Err("Give the fact a short name, like tax-filing.".into());
    }
    let description = one_line(fact.description);
    if description.is_empty() {
        return Err("Describe the fact in one line: it is what everyone sees in the index.".into());
    }
    let kind = if KINDS.contains(&fact.kind) { fact.kind } else { "other" };
    if let Some(what) = sensitive(&format!("{name}\n{description}\n{}", fact.body)) {
        return Err(format!(
            "That looks like {what}. The squad's memory never keeps such values: save the original as a file in the \
squad's folder (or leave it where it is) and remember only where it is."
        ));
    }
    squad_home::save_in(folder, PERSON, "changed the squad's memory", &[DIR])?;
    let file = file_for(folder, name);
    let path = dir(folder).join(&file);
    let existed = path.exists();
    if existed && !fact.confirmed && last_by(folder, &file).as_deref() == Some(PERSON) {
        return Err(format!(
            "The person last changed {DIR}/{file} themselves, and what they wrote wins. Read it; if it still should \
change, ask them, and call again with confirmed: true once they agree."
        ));
    }
    let body = fact.body.trim();
    let text = format!(
        "---\nname: {name}\ndescription: {description}\nkind: {kind}\nupdated: {today}\nby: {by}\n---\n\n{}",
        if body.is_empty() { format!("{description}\n") } else { format!("{body}\n") }
    );
    std::fs::create_dir_all(dir(folder)).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    refresh_index(folder, &facts(folder), language);
    let why = format!("{} {name}: {description}", if existed { "updated" } else { "remembered" });
    let change = squad_home::save_in(folder, by, &why, &[DIR])?.ok_or("Nothing changed: the squad already remembers exactly that.")?;
    Ok((change, !existed))
}

/// Forget a fact that turned out wrong or stale, and save that, by `by`.
pub fn forget(folder: &Path, by: &str, name: &str, language: Option<&str>) -> Result<Change, String> {
    squad_home::save_in(folder, PERSON, "changed the squad's memory", &[DIR])?;
    let file = file_for(folder, name.trim());
    let path = dir(folder).join(&file);
    if !path.exists() {
        return Err(format!("The squad remembers nothing called {name}."));
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    refresh_index(folder, &facts(folder), language);
    squad_home::save_in(folder, by, &format!("forgot {}", name.trim()), &[DIR])?.ok_or_else(|| "Nothing changed.".into())
}

/// The latest `n` changes to the memory, newest first.
pub fn changes(folder: &Path, n: usize) -> Result<Vec<Change>, String> {
    squad_home::changes_in(folder, n, &[DIR])
}

/// Who last changed a fact's file.
fn last_by(folder: &Path, file: &str) -> Option<String> {
    let path = format!("{DIR}/{file}");
    squad_home::changes_in(folder, 1, &[&path]).ok()?.into_iter().next().map(|c| c.by)
}

fn one_line(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// What kind of secret value `text` seems to hold, if any: a resident
/// registration number, a card number (Luhn), a bank account number. Phone
/// numbers and dates pass. A safety net for what members write, not a filter
/// for what they read.
pub fn sensitive(text: &str) -> Option<&'static str> {
    // Runs of digits joined by single dashes or spaces: "900101-1234567", "110-123-456789".
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut groups: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            current.push(c);
            continue;
        }
        let joins = (c == '-' || c == ' ') && !current.is_empty() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        if !joins && !groups.is_empty() {
            runs.push(std::mem::take(&mut groups));
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    if !groups.is_empty() {
        runs.push(groups);
    }
    for groups in runs {
        let digits: String = groups.concat();
        let lens: Vec<usize> = groups.iter().map(String::len).collect();
        let rrn = |g: &[String]| g.len() == 2 && g[0].len() == 6 && g[1].len() == 7 && matches!(g[1].as_bytes()[0], b'1'..=b'8');
        if rrn(&groups) || (lens == [13] && matches!(digits.as_bytes()[6], b'1'..=b'8') && valid_birth(&digits[..6])) {
            return Some("a resident registration number");
        }
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            return Some("a card number");
        }
        let phone = digits.starts_with('0') && digits.len() <= 11 && groups.len() == 3 && lens[0] <= 3;
        let date = groups.len() == 3 && lens[0] == 4 && lens[1] <= 2 && lens[2] <= 2;
        if !phone && !date && groups.len() >= 3 && (10..=16).contains(&digits.len()) {
            return Some("a bank account number");
        }
    }
    None
}

fn valid_birth(yymmdd: &str) -> bool {
    let month: u32 = yymmdd[2..4].parse().unwrap_or(0);
    let day: u32 = yymmdd[4..6].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 1 { if d * 2 > 9 { d * 2 - 9 } else { d * 2 } } else { d }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Today as `YYYY-MM-DD` (UTC), from unix seconds.
pub fn date(secs: u64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn folder() -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("equipe-memory-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        squad_home::ensure(&home, "acme", None).unwrap();
        squad_home::path(&home, "acme")
    }

    fn tax<'a>() -> Remember<'a> {
        Remember { name: "tax-filing", description: "사람은 매년 5월에 종합소득세를 직접 신고한다", kind: "deadline", body: "", confirmed: false }
    }

    #[test]
    fn a_fact_is_one_file_with_its_frontmatter_and_the_index_follows() {
        let folder = folder();
        let (change, new) = remember(&folder, "backend", &tax(), "2026-10-13", Some("Korean")).unwrap();
        assert!(new);
        assert_eq!(change.by, "backend");
        assert_eq!(change.what(), "saved memory/tax-filing.md", "the index is never news");
        let text = std::fs::read_to_string(dir(&folder).join("tax-filing.md")).unwrap();
        assert!(text.starts_with("---\nname: tax-filing\ndescription: 사람은 매년 5월에"), "{text}");
        assert!(text.contains("kind: deadline\nupdated: 2026-10-13\nby: backend\n---"), "{text}");
        let index = std::fs::read_to_string(dir(&folder).join(INDEX)).unwrap();
        assert!(index.contains("# 스쿼드 기억"), "{index}");
        assert!(index.contains("- [tax-filing](tax-filing.md) (deadline, 2026-10-13) — 사람은 매년 5월에 종합소득세를 직접 신고한다"), "{index}");

        // The same name updates it rather than making another.
        let again = Remember { description: "사람은 매년 5월 말까지 종합소득세를 직접 신고한다", ..tax() };
        let (change, new) = remember(&folder, "pm", &again, "2026-10-14", None).unwrap();
        assert!(!new);
        assert_eq!(change.what(), "updated memory/tax-filing.md");
        assert_eq!(facts(&folder).len(), 1);
        assert!(remember(&folder, "pm", &again, "2026-10-14", None).is_err(), "nothing new, nothing saved");

        let gone = forget(&folder, "pm", "tax-filing", None).unwrap();
        assert_eq!(gone.what(), "removed memory/tax-filing.md");
        assert!(facts(&folder).is_empty());
        assert!(forget(&folder, "pm", "tax-filing", None).unwrap_err().contains("nothing called"));
        assert_eq!(changes(&folder, 10).unwrap().len(), 3);
    }

    #[test]
    fn what_the_person_wrote_last_wins_until_they_agree() {
        let folder = folder();
        remember(&folder, "backend", &tax(), "2026-10-13", None).unwrap();
        // The person fixes it in Finder, and drops in a note of their own without frontmatter.
        let path = dir(&folder).join("tax-filing.md");
        std::fs::write(&path, std::fs::read_to_string(&path).unwrap().replace("직접 신고", "세무사가 신고")).unwrap();
        std::fs::write(dir(&folder).join("dentist.md"), "# 치과는 화요일 오후만\n").unwrap();
        let listed = facts(&folder);
        assert_eq!(listed[0], Fact { file: "dentist.md".into(), name: "dentist".into(), description: "치과는 화요일 오후만".into(), kind: "other".into(), updated: String::new(), body: "# 치과는 화요일 오후만".into() });
        assert!(listed[1].description.contains("세무사가"));

        let err = remember(&folder, "pm", &tax(), "2026-10-14", None).unwrap_err();
        assert!(err.contains("what they wrote wins"), "{err}");
        // Their edit was saved as theirs before anything else.
        let latest = changes(&folder, 1).unwrap().remove(0);
        assert_eq!(latest.by, PERSON);
        assert!(latest.files.iter().any(|(_, f)| f == "memory/tax-filing.md"));
        remember(&folder, "pm", &Remember { confirmed: true, ..tax() }, "2026-10-14", None).unwrap();
    }

    #[test]
    fn secret_values_are_never_remembered() {
        let folder = folder();
        for (body, what) in [
            ("주민번호 900101-1234567", "resident registration"),
            ("카드 4111 1111 1111 1111", "card"),
            ("국민은행 123456-01-234567", "bank account"),
            ("신한 110-123-456789", "bank account"),
        ] {
            let err = remember(&folder, "pm", &Remember { body, ..tax() }, "2026-10-13", None).unwrap_err();
            assert!(err.contains(what), "{body}: {err}");
        }
        for fine in ["전화 010-1234-5678", "마감 2026-05-31", "02-123-4567", "원본: outputs/T-12 신분증/id.pdf", "T-31, 3시"] {
            assert_eq!(sensitive(fine), None, "{fine}");
        }
        assert!(facts(&folder).is_empty());
    }

    #[test]
    fn the_charter_gets_the_index_capped() {
        let many: Vec<Fact> = (0..SHOWN + 2).map(|i| parse(&format!("f{i:03}.md"), &format!("fact {i}"))).collect();
        let text = for_charter(&many).unwrap();
        assert_eq!(text.lines().count(), SHOWN + 1);
        assert!(text.ends_with("… and 2 more: see memory/INDEX.md"), "{text}");
        assert_eq!(for_charter(&[]), None);
    }

    #[test]
    fn names_and_dates() {
        assert_eq!(slug("Tax filing / 5월"), "tax-filing-5월");
        assert_eq!(slug("  --  "), "");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_791_849_600), "2026-10-13");
    }
}
