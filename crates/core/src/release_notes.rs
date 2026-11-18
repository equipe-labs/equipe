//! equipe's own release notes, in the words of the people who use it.
//!
//! At release time (`.github/workflows/release.yml`, `equipe release-notes`)
//! each pull request merged since the last release gives its lines: the
//! bullets under a `## For users` heading in its description when it has
//! one (an empty section: nothing for users), else its title — internal
//! work (`ci:`, `test:`, `chore:` …, or the `internal` label) gives none.
//! They go at the head of the GitHub release, and into `release-notes.json`
//! beside the appcasts: the last [`KEPT`] releases' notes, newest first
//! ([`Feed`]), which the app reads to say what a waiting update brings and,
//! after a restart, what the update brought ([`between`]).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// The file the app reads, next to the appcasts of the latest release.
pub const FILE: &str = "release-notes.json";
/// Releases a feed keeps, the newest.
pub const KEPT: usize = 40;
/// The heading whose lines are a pull request's notes for users.
const HEADING: &str = "for users";
/// Labels that keep a pull request out of the notes.
const QUIET_LABELS: [&str; 4] = ["internal", "skip-changelog", "no-release-notes", "dependencies"];
/// Conventional title prefixes of work nobody using equipe sees.
const INTERNAL_PREFIXES: [&str; 8] = ["ci", "test", "tests", "chore", "docs", "refactor", "build", "style"];
/// Conventional title prefixes dropped from a note that people do see.
const SHOWN_PREFIXES: [&str; 5] = ["feat", "feature", "fix", "perf", "revert"];

/// A pull request, as `gh pr view --json number,title,body,labels` gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Pr {
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<Label>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Label {
    pub name: String,
}

/// One release's notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, UTC.
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// The notes of the latest releases, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    #[serde(default)]
    pub releases: Vec<Release>,
}

/// The lines under a pull request description's `## For users` heading (any
/// level): `None` when it has no such heading; an empty list when the
/// section says nothing (or `None`, `n/a`, `-`). Bullets each make a line,
/// as does each paragraph of plain text; comments are left out.
pub fn for_users(body: &str) -> Option<Vec<String>> {
    let mut lines = body.lines();
    let level = loop {
        let line = lines.next()?;
        if let Some((level, title)) = heading(line)
            && title.trim().trim_end_matches(':').eq_ignore_ascii_case(HEADING)
        {
            break level;
        }
    };
    let mut notes: Vec<String> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut comment = false;
    let flush = |paragraph: &mut Vec<&str>, notes: &mut Vec<String>| {
        if !paragraph.is_empty() {
            notes.push(paragraph.join(" "));
            paragraph.clear();
        }
    };
    for line in lines {
        if let Some((l, _)) = heading(line) {
            if l <= level {
                break;
            }
            // A sub-heading inside it: its lines count, not its words.
            flush(&mut paragraph, &mut notes);
            continue;
        }
        let mut text = line.trim();
        // An HTML comment, a template's hint: never a note.
        if comment || text.starts_with("<!--") {
            comment = !text.contains("-->");
            continue;
        }
        if text.is_empty() {
            flush(&mut paragraph, &mut notes);
            continue;
        }
        let bullet = ["- ", "* ", "+ "].iter().find_map(|b| text.strip_prefix(b)).or_else(|| {
            let digits = text.chars().take_while(char::is_ascii_digit).count();
            (digits > 0).then(|| text[digits..].strip_prefix(". ").or_else(|| text[digits..].strip_prefix(") "))).flatten()
        });
        if let Some(rest) = bullet {
            flush(&mut paragraph, &mut notes);
            text = rest.trim();
            if !text.is_empty() {
                notes.push(text.to_owned());
            }
        } else if let Some(last) = notes.last_mut().filter(|_| paragraph.is_empty() && line.starts_with([' ', '\t'])) {
            // A bullet's line carried on below it.
            last.push(' ');
            last.push_str(text);
        } else {
            paragraph.push(text);
        }
    }
    flush(&mut paragraph, &mut notes);
    notes.retain(|n| !matches!(n.trim().to_lowercase().as_str(), "none" | "n/a" | "-" | "nothing" | "none."));
    Some(notes)
}

/// A Markdown heading's level and words.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes).then(|| line[hashes..].strip_prefix(' ').map(|rest| (hashes, rest.trim()))).flatten()
}

/// A pull request's title as a note: its conventional prefix (`fix:`,
/// `feat(desktop):`) and its number dropped, its first letter raised.
/// `None` for work nobody using equipe sees (`ci:`, `chore:`, `test:` …).
pub fn title_note(title: &str) -> Option<String> {
    let mut text = title.trim();
    if let Some(open) = text.rfind(" (#")
        && text.ends_with(')')
    {
        text = text[..open].trim_end();
    }
    if let Some((prefix, rest)) = text.split_once(':') {
        let kind = prefix.split('(').next().unwrap_or(prefix).trim().trim_end_matches('!').to_ascii_lowercase();
        if INTERNAL_PREFIXES.contains(&kind.as_str()) {
            return None;
        }
        if SHOWN_PREFIXES.contains(&kind.as_str()) {
            text = rest.trim();
        }
    }
    let mut chars = text.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// The notes of the pull requests a release brings, in their order, each
/// line once.
pub fn notes_of(prs: &[Pr]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for pr in prs {
        if pr.labels.iter().any(|l| QUIET_LABELS.contains(&l.name.to_ascii_lowercase().as_str())) {
            continue;
        }
        let lines = for_users(&pr.body).unwrap_or_else(|| title_note(&pr.title).into_iter().collect());
        for line in lines {
            if !out.contains(&line) {
                out.push(line);
            }
        }
    }
    out
}

/// Versions in order: their numbers one by one; a pre-release (`-rc.1`)
/// before its release.
pub fn compare(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (Vec<u64>, Option<String>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p.to_owned())));
        (core.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    let len = na.len().max(nb.len());
    for i in 0..len {
        match na.get(i).unwrap_or(&0).cmp(nb.get(i).unwrap_or(&0)) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    }
}

/// `release` in `feed`, in place of one of the same version, newest first,
/// the newest [`KEPT`] kept.
pub fn add(mut feed: Feed, release: Release) -> Feed {
    feed.releases.retain(|r| compare(&r.version, &release.version) != Ordering::Equal);
    feed.releases.push(release);
    feed.releases.sort_by(|a, b| compare(&b.version, &a.version));
    feed.releases.truncate(KEPT);
    feed
}

/// A release's notes at the head of its GitHub release. Empty when it has none.
pub fn markdown(release: &Release) -> String {
    if release.notes.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = release.notes.iter().map(|n| format!("- {n}")).collect();
    format!("## What's new\n\n{}\n", lines.join("\n"))
}

/// The releases after `after` (none: only `upto`'s own) up to `upto`, newest first.
pub fn between<'a>(feed: &'a Feed, after: Option<&str>, upto: &str) -> Vec<&'a Release> {
    feed.releases
        .iter()
        .filter(|r| compare(&r.version, upto) != Ordering::Greater)
        .filter(|r| match after {
            Some(after) => compare(&r.version, after) == Ordering::Greater,
            None => compare(&r.version, upto) == Ordering::Equal,
        })
        .collect()
}

/// The first `most` notes of `releases` (newest first), and how many more there are.
pub fn summary(releases: &[&Release], most: usize) -> (Vec<String>, usize) {
    let mut all: Vec<&String> = Vec::new();
    for note in releases.iter().flat_map(|r| &r.notes) {
        if !all.contains(&note) {
            all.push(note);
        }
    }
    let shown = all.iter().take(most).map(|n| n.to_string()).collect();
    (shown, all.len().saturating_sub(most))
}

/// Where the feed is, beside an appcast (`…/releases/latest/download/appcast-arm64.xml`).
pub fn feed_url(appcast: &str) -> Option<String> {
    let (folder, file) = appcast.rsplit_once('/')?;
    (file.ends_with(".xml") && folder.contains("://")).then(|| format!("{folder}/{FILE}"))
}

/// The feed at `url`. Blocks: call it on a thread.
pub fn fetch(url: &str) -> Result<Feed, String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client.get(url).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("{url}: {}", response.status()));
        }
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{url}: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(title: &str, body: &str, labels: &[&str]) -> Pr {
        Pr { number: 1, title: title.into(), body: body.into(), labels: labels.iter().map(|l| Label { name: l.to_string() }).collect() }
    }

    fn release(version: &str, notes: &[&str]) -> Release {
        Release { version: version.into(), date: "2026-11-18".into(), notes: notes.iter().map(|n| n.to_string()).collect() }
    }

    #[test]
    fn a_pull_requests_for_users_section_gives_its_lines() {
        let body = "Fixes the hang.\n\n## For users\n\n<!-- What people notice, in their words. -->\n- The sidebar shows what went live\n- Restart to Update says what's new,\n  in a line or two\n1. Numbered works too\n\nA plain paragraph\nover two lines.\n\n## Testing\n- not this\n";
        assert_eq!(
            for_users(body).unwrap(),
            ["The sidebar shows what went live", "Restart to Update says what's new, in a line or two", "Numbered works too", "A plain paragraph over two lines."]
        );
        assert_eq!(for_users("### for users:\n- Deeper heading\n#### a sub-heading\n- still in it\n## Next\n- out").unwrap(), ["Deeper heading", "still in it"]);
        assert_eq!(for_users("## For users\n\nNone\n\n## Why\nx").unwrap(), Vec::<String>::new(), "said: nothing for users");
        assert_eq!(for_users("## For users\n<!--\nmulti-line\nhint\n-->\n"), Some(vec![]), "the template left as it is");
        assert_eq!(for_users("## Summary\n- users won't see this"), None);
        assert_eq!(for_users("#For users\n- not a heading without its space"), None);
    }

    #[test]
    fn a_title_is_the_note_when_there_is_no_section_and_internal_work_gives_none() {
        assert_eq!(title_note("fix(desktop): keep the sidebar's width (#41)").as_deref(), Some("Keep the sidebar's width"));
        assert_eq!(title_note("feat!: what went live").as_deref(), Some("What went live"));
        assert_eq!(title_note("Show what went live").as_deref(), Some("Show what went live"));
        assert_eq!(title_note("Sidebar: show releases").as_deref(), Some("Sidebar: show releases"), "only conventional prefixes go");
        assert_eq!(title_note("ci: pin codex"), None);
        assert_eq!(title_note("chore(deps): bump"), None);
        assert_eq!(title_note("test: retry the flaky lane"), None);
        let prs = vec![
            pr("feat: releases feed", "## For users\n- See what went live in the sidebar\n", &[]),
            pr("Fix the Windows accept test hang", "", &["internal"]),
            pr("ci: pin codex", "", &[]),
            pr("Faster merges", "No section here", &[]),
            pr("Docs only", "## For users\nnone\n", &[]),
            pr("Same again", "## For users\n- See what went live in the sidebar\n", &[]),
        ];
        assert_eq!(notes_of(&prs), ["See what went live in the sidebar", "Faster merges"]);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert_eq!(compare("0.103.138", "0.103.99"), Ordering::Greater);
        assert_eq!(compare("v1.2.0", "1.2"), Ordering::Equal);
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(compare("0.103.137", "0.104.0"), Ordering::Less);
    }

    #[test]
    fn the_feed_keeps_the_newest_and_says_what_an_update_brings() {
        let mut feed = Feed::default();
        for v in ["0.103.136", "0.103.138", "0.103.137"] {
            feed = add(feed, release(v, &[&format!("in {v}")]));
        }
        feed = add(feed, release("0.103.138", &["New input box", "in 0.103.137"]));
        assert_eq!(feed.releases.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), ["0.103.138", "0.103.137", "0.103.136"]);
        // From 0.103.136 to 0.103.138: the two after it.
        let brought = between(&feed, Some("0.103.136"), "0.103.138");
        assert_eq!(brought.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), ["0.103.138", "0.103.137"]);
        assert_eq!(summary(&brought, 1), (vec!["New input box".to_owned()], 1), "a line in both is counted once");
        assert_eq!(between(&feed, None, "0.103.137").len(), 1, "unknown before: its own notes only");
        assert!(between(&feed, Some("0.103.138"), "0.103.138").is_empty());
        for i in 0..KEPT + 3 {
            feed = add(feed, release(&format!("1.0.{i}"), &[]));
        }
        assert_eq!(feed.releases.len(), KEPT);
        assert_eq!(feed.releases[0].version, format!("1.0.{}", KEPT + 2));
        let json = serde_json::to_string(&feed).unwrap();
        assert_eq!(serde_json::from_str::<Feed>(&json).unwrap(), feed);
        assert_eq!(serde_json::from_str::<Feed>("{}").unwrap(), Feed::default(), "an empty file reads");
    }

    #[test]
    fn the_release_body_and_the_feeds_address() {
        assert_eq!(markdown(&release("1.0.0", &["A", "B"])), "## What's new\n\n- A\n- B\n");
        assert_eq!(markdown(&release("1.0.0", &[])), "");
        assert_eq!(
            feed_url("https://github.com/equipe-labs/equipe/releases/latest/download/appcast-arm64.xml").as_deref(),
            Some("https://github.com/equipe-labs/equipe/releases/latest/download/release-notes.json")
        );
        assert_eq!(feed_url("appcast.xml"), None);
    }
}
