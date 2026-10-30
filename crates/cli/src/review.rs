//! `equipe review`: an NVIDIA Nemotron model on Nebius Token Factory reviews
//! this repository's changes — the working tree and what is staged against
//! `HEAD`, or with `--base <ref>` the commits since `<ref>` — the way the
//! squad's hand-ins are reviewed (`equipe_core::nemotron`). Only a Nebius
//! key is needed (`NEBIUS_API_KEY`): no squad, no daemon, no engine.
//! `--models` lists the NVIDIA Nemotron models the account offers.

use std::path::Path;
use std::process::Command;

use equipe_core::nemotron::{self, Config, ReviewInput};
use equipe_core::protocol::review::{CodeReview, ReviewVerdict};
use equipe_core::t;

use crate::{Failure, ui};

pub async fn command(base: Option<String>, list_models: bool, goal: Option<String>) -> Result<(), Failure> {
    let lang = crate::i18n::lang();
    let mut cfg = Config::from_env().map_err(|err| Failure::new(err.why(lang)).next("export NEBIUS_API_KEY=…"))?;
    let failed = |err: nemotron::ReviewError| Failure::new(t!("review.failed", why = err.why(lang)));
    if list_models {
        let offered = nemotron::nemotron_models(&nemotron::models(&cfg).await.map_err(failed)?);
        let Some(first) = offered.first() else {
            return Err(failed(nemotron::ReviewError::NoModel));
        };
        let p = ui::out();
        println!("{}", p.kicker(t!("review.models_title")));
        for id in &offered {
            println!("  {id}");
        }
        let used = cfg.model.clone().unwrap_or_else(|| first.clone());
        println!("\n{}", p.dim(&t!("review.models_used", model = used)));
        return Ok(());
    }
    let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
    let root = git(&cwd, &["rev-parse", "--show-toplevel"]).map_err(|_| Failure::new(t!("review.not_a_repo")))?;
    let root = Path::new(root.trim());
    let (diff, title, body) = match &base {
        Some(base) => {
            let range = format!("{base}...HEAD");
            let diff = git(root, &["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", &range]).map_err(|err| Failure::new(t!("review.git_failed", err = err)))?;
            // What the commits say they do, when nothing else says.
            let log = git(root, &["log", "--no-color", "--format=- %s", &format!("{base}..HEAD")]).unwrap_or_default();
            (diff, format!("Changes since {base}"), goal.unwrap_or(log))
        }
        None => {
            let diff = git(root, &["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", "HEAD"]).map_err(|err| Failure::new(t!("review.git_failed", err = err)))?;
            (diff, "Uncommitted changes".to_owned(), goal.unwrap_or_default())
        }
    };
    if diff.trim().is_empty() {
        let against = base.unwrap_or_else(|| "HEAD".into());
        return Err(Failure::new(t!("review.nothing", against = against)));
    }
    // The model, named before the wait.
    if cfg.model.is_none() {
        cfg.model = Some(nemotron::pick_model(&nemotron::models(&cfg).await.map_err(failed)?).map_err(failed)?);
    }
    let model = cfg.model.clone().unwrap_or_default();
    let what = match &base {
        Some(base) => t!("review.what_base", base = base),
        None => t!("review.what_worktree").to_owned(),
    };
    eprintln!("{}", ui::err().dim(&t!("review.asking", model = model, what = what)));
    let input = ReviewInput { ticket: 0, title, body, diff, language: lang.english_name().to_owned() };
    let reviewed = nemotron::review_detailed(&cfg, input).await.map_err(failed)?;
    for line in lines(&reviewed.review) {
        println!("{line}");
    }
    let mut facts = Vec::new();
    if let Some(usage) = reviewed.usage {
        facts.push(t!("review.tokens", prompt = usage.prompt, completion = usage.completion));
    }
    facts.push(t!("review.took", seconds = format!("{:.1}", reviewed.latency.as_secs_f64())));
    if let Some(id) = &reviewed.request_id {
        facts.push(t!("review.request", id = id));
    }
    println!("\n{}", ui::out().dim(&facts.join(" · ")));
    Ok(())
}

/// A review as the terminal shows it.
pub fn lines(review: &CodeReview) -> Vec<String> {
    let p = ui::out();
    let mut out = vec![p.bold(&t!("review.kicker", model = review.model)), String::new()];
    let verdict = verdict_word(review.verdict);
    let verdict = if review.verdict == ReviewVerdict::LooksGood { p.ok(verdict) } else { p.fail(verdict) };
    out.push(format!("{}  {}", verdict, review.summary));
    out.push(String::new());
    out.push(p.bold(t!("review.risks")));
    if review.risks.is_empty() {
        out.push(format!("  {}", p.dim(t!("review.none"))));
    }
    for risk in &review.risks {
        let place = match (risk.file.as_str(), risk.line) {
            ("", _) => String::new(),
            (file, Some(line)) => format!("{file}:{line} — "),
            (file, None) => format!("{file} — "),
        };
        out.push(ui::wrap(&format!("  • {place}{}", risk.why), 0, 4, ui::width()));
    }
    out.push(String::new());
    out.push(p.bold(t!("review.missing_tests")));
    if review.missing_tests.is_empty() {
        out.push(format!("  {}", p.dim(t!("review.none"))));
    }
    for test in &review.missing_tests {
        out.push(ui::wrap(&format!("  • {test}"), 0, 4, ui::width()));
    }
    if review.truncated {
        out.push(String::new());
        out.push(p.dim(&t!("review.truncated", chars = nemotron::MAX_DIFF_CHARS)));
    }
    out
}

pub fn verdict_word(verdict: ReviewVerdict) -> &'static str {
    match verdict {
        ReviewVerdict::LooksGood => t!("review.verdict.looks_good"),
        ReviewVerdict::NeedsAttention => t!("review.verdict.needs_attention"),
        ReviewVerdict::Risky => t!("review.verdict.risky"),
    }
}

/// What git printed in `dir`, or what it said went wrong.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").output().map_err(|err| err.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::review::ReviewPoint;

    #[test]
    fn a_review_reads_verdict_summary_risks_and_missing_tests() {
        let review = CodeReview {
            ticket: 0,
            model: "nvidia/mock-nemotron".into(),
            verdict: ReviewVerdict::Risky,
            summary: "A failed lookup lets the request through.".into(),
            risks: vec![ReviewPoint { file: "src/auth.rs".into(), line: Some(42), why: "Returns Ok on error.".into() }, ReviewPoint { file: String::new(), line: None, why: "No rate limit.".into() }],
            missing_tests: vec![],
            truncated: true,
            at: 0,
        };
        let text = lines(&review).join("\n");
        assert!(text.contains("Nemotron review · nvidia/mock-nemotron"), "{text}");
        assert!(text.contains("Risky") && text.contains("A failed lookup lets the request through."), "{text}");
        assert!(text.contains("• src/auth.rs:42 — Returns Ok on error.") && text.contains("• No rate limit."), "{text}");
        assert!(text.contains("Missing tests\n  none"), "{text}");
        assert!(text.contains("48000 characters"), "{text}");
    }
}
