//! Habits kept on purpose. The design's decision log (`design/decisions/`,
//! one Markdown file a decision) holds them as a line each:
//!
//! `Waive tracked-caps: the brand sets its labels in capitals (brand.md, Locked)`
//! `Waive nested-cards (settings): the billing summary is a card in a card by design`
//!
//! — the rule, optionally the screen or file it holds for, and the reason.
//! A floor rule is never waived: such a line is refused, and said. A rule
//! about building from the design system (`system-component`, …) is kept
//! the same way as a habit — the line is its allow-list entry, scoped to the
//! file or screen where the exception is real:
//!
//! `Waive system-component (src/app/legacy/Modal.tsx): the old checkout stays until T-40 moves it to Dialog`

use std::path::{Path, PathBuf};

use super::Finding;
use super::rules::{self, Tier};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiver {
    pub rule: String,
    /// The screen or file it holds for; everywhere when none.
    pub scope: Option<String>,
    pub why: String,
    /// The file and line it is written on.
    pub at: String,
}

impl Waiver {
    pub fn covers(&self, f: &Finding, screen: Option<&str>) -> bool {
        f.rule == self.rule
            && matches!(f.tier, Tier::Habit | Tier::System)
            && self.scope.as_deref().is_none_or(|s| screen.is_some_and(|x| x == s || x.ends_with(&format!("/{s}"))) || f.file.as_deref().is_some_and(|file| file.contains(s)))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Log {
    pub waivers: Vec<Waiver>,
    /// Lines that try to waive what can't be, and why not.
    pub refused: Vec<String>,
}

/// The decision log's folder.
pub fn dir(design: &Path) -> PathBuf {
    design.join("decisions")
}

pub fn read(design: &Path) -> Log {
    let mut log = Log::default();
    let Ok(entries) = std::fs::read_dir(dir(design)) else { return log };
    let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        parse(&text, &name, &mut log);
    }
    log
}

/// The waivers written in one decision file.
pub fn parse(text: &str, name: &str, log: &mut Log) {
    for (n, line) in text.lines().enumerate() {
        let at = format!("decisions/{name}:{}", n + 1);
        let trimmed = line.trim().trim_start_matches(['-', '*', ' ']).trim();
        let lower = trimmed.to_ascii_lowercase();
        let Some(rest) = ["waive ", "waived ", "waiver "].iter().find_map(|p| lower.starts_with(p).then(|| &trimmed[p.len()..])) else { continue };
        let Some((head, why)) = rest.split_once(':') else {
            log.refused.push(format!("{at}: a waiver says which rule and why: `Waive <rule>: <reason>`"));
            continue;
        };
        let head = head.trim().trim_matches('`');
        let (rule, scope) = match head.split_once('(') {
            Some((rule, scope)) => (rule.trim().trim_matches('`'), Some(scope.trim_end_matches(')').trim().to_owned()).filter(|s| !s.is_empty())),
            None => (head, None),
        };
        let why = why.trim();
        match rules::rule(rule) {
            None => log.refused.push(format!("{at}: no rule called `{rule}`")),
            Some(r) if r.tier == Tier::Floor => log.refused.push(format!("{at}: `{rule}` is a floor rule and is never waived — fix it")),
            Some(r) if r.tier == Tier::Finish => {}
            Some(_) if why.chars().filter(|c| c.is_alphanumeric()).count() < 8 => log.refused.push(format!("{at}: a habit is kept only with its reason — say why `{rule}` stays")),
            Some(r) => log.waivers.push(Waiver { rule: r.id.to_owned(), scope, why: why.to_owned(), at }),
        }
    }
}

/// A decision file keeping `rule`, as `equipe design lint --waive` writes it.
pub fn decision(rule: &str, scope: Option<&str>, why: &str, ticket: Option<&str>) -> Result<String, String> {
    let r = rules::rule(rule).ok_or_else(|| format!("no rule called `{rule}`"))?;
    if r.tier == Tier::Floor {
        return Err(format!("`{rule}` is a floor rule and is never waived — fix it"));
    }
    if why.chars().filter(|c| c.is_alphanumeric()).count() < 8 {
        return Err("say why it stays (--why \"…\")".into());
    }
    let scope_text = scope.map(|s| format!(" ({s})")).unwrap_or_default();
    let mut out = format!("# Keep `{rule}`{scope_text}?\n\nChose: keep it — {}\nWhy: {why}\n", r.checks.trim_end_matches('.'));
    if let Some(ticket) = ticket {
        out.push_str(&format!("Ticket: {ticket}\n"));
    }
    out.push_str(&format!("\nWaive {rule}{scope_text}: {why}\n"));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_habit_is_kept_with_its_reason_and_a_floor_rule_never() {
        let mut log = Log::default();
        parse(
            "# Labels\nChose: capitals\n\nWaive tracked-caps: the brand sets labels in capitals, locked in brand.md\n- Waive `nested-cards` (billing): the invoice is a card on a card by design\nWaive contrast: it looks nicer\nWaive emoji-icons: ok\nWaive nope: because reasons\nWaive line-length: finish notes need no waiver at all\n",
            "2026-11-01-labels.md",
            &mut log,
        );
        assert_eq!(log.waivers.len(), 2, "{log:#?}");
        assert_eq!(log.waivers[0].rule, "tracked-caps");
        assert_eq!(log.waivers[0].at, "decisions/2026-11-01-labels.md:4");
        assert_eq!(log.waivers[1].scope.as_deref(), Some("billing"));
        assert!(log.refused.iter().any(|r| r.contains("`contrast` is a floor rule")), "{:?}", log.refused);
        assert!(log.refused.iter().any(|r| r.contains("say why `emoji-icons`")), "{:?}", log.refused);
        assert!(log.refused.iter().any(|r| r.contains("no rule called `nope`")));
        let f = |rule: &'static str, file: &str| Finding::new(rule, file, Some(1), "x");
        assert!(log.waivers[1].covers(&f("nested-cards", "design/mockups/billing.html"), None));
        assert!(!log.waivers[1].covers(&f("nested-cards", "design/mockups/home.html"), None));
        assert!(log.waivers[1].covers(&f("nested-cards", "x.html"), Some("account/billing")));
        assert!(log.waivers[0].covers(&f("tracked-caps", "any.css"), None));
    }

    #[test]
    fn the_decision_written_for_a_waiver_reads_back_as_one() {
        let text = decision("all-centred", Some("welcome"), "a first-run screen with one message and one action", Some("T-12")).unwrap();
        let mut log = Log::default();
        parse(&text, "x.md", &mut log);
        assert_eq!(log.waivers.len(), 1);
        assert!(log.refused.is_empty());
        assert!(text.contains("Ticket: T-12"));
        assert!(decision("contrast", None, "it is a long enough reason", None).unwrap_err().contains("never waived"));
        assert!(decision("all-centred", None, "ok", None).is_err());
    }
}
