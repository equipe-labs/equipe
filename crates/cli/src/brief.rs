//! `equipe brief critique <file>`: a brief the PM wrote after office hours
//! goes to the other engine for a sceptical second opinion — a second model's
//! reading, not the author's own (`equipe_core::briefs`). Run from a Claude
//! member, Codex reads it; from a Codex member, Claude. The critic only
//! reads; its answer is printed for the PM to fold in.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::Subcommand;
use equipe_core::briefs;
use equipe_core::design::critique::{caller_engine, critic_engine};
use equipe_core::protocol::Engine;

#[derive(Subcommand)]
pub enum BriefAction {
    /// Have the other engine read a brief sceptically: gaps, blind spots, settle or revise
    Critique {
        /// The brief: docs/briefs/<date>-<slug>.md
        file: PathBuf,
        /// Who reads it (default: the engine this isn't run on)
        #[arg(long, value_parser = ["codex", "claude"])]
        engine: Option<String>,
    },
}

pub async fn command(action: BriefAction) -> Result<(), String> {
    match action {
        BriefAction::Critique { file, engine } => critique(&file, engine.as_deref()).await,
    }
}

/// Who reads it: the one named, else the engine the caller isn't on.
fn engine_for(named: Option<&str>, env: impl Fn(&str) -> Option<String>) -> Engine {
    match named {
        Some("claude") => Engine::Claude,
        Some(_) => Engine::Codex,
        None => critic_engine(caller_engine(env)),
    }
}

async fn critique(file: &Path, engine: Option<&str>) -> Result<(), String> {
    let meta = std::fs::metadata(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if !meta.is_file() || meta.len() > briefs::MAX {
        return Err(format!("{}: not a brief (a Markdown file of at most {} KB)", file.display(), briefs::MAX >> 10));
    }
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let engine = engine_for(engine, |name| std::env::var(name).ok());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let answer: PathBuf = std::env::temp_dir().join(format!("equipe-brief-critique-{}-{stamp}.md", std::process::id()));
    let (program, args) = briefs::critique_command(engine, &text, file, &answer);
    eprintln!("{} is reading {}…", engine.label(), file.display());
    let run = tokio::process::Command::new(&program).args(&args).stdin(Stdio::null()).stderr(Stdio::piped()).output().await.map_err(|e| format!("{program}: {e} — is {} installed and signed in?", engine.label()))?;
    if !run.status.success() {
        let _ = std::fs::remove_file(&answer);
        // Claude says why on stdout, Codex on stderr.
        let said = [String::from_utf8_lossy(&run.stderr), String::from_utf8_lossy(&run.stdout)].join("\n");
        let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(6).collect();
        return Err(format!("{} couldn't read it ({}): {}", engine.label(), run.status, tail.into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    // Codex wrote its answer to the file; Claude printed it.
    let said = match engine {
        Engine::Codex => std::fs::read_to_string(&answer).unwrap_or_default(),
        Engine::Claude => String::from_utf8_lossy(&run.stdout).into_owned(),
    };
    let _ = std::fs::remove_file(&answer);
    if said.trim().is_empty() {
        return Err(format!("{} answered nothing", engine.label()));
    }
    println!("{}", said.trim_end());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_the_caller_isnt_on_reads_the_brief_unless_one_is_named() {
        let claude = |name: &str| (name == "CLAUDECODE").then(|| "1".to_owned());
        assert_eq!(engine_for(None, claude), Engine::Codex);
        assert_eq!(engine_for(None, |name: &str| (name == "CODEX_THREAD_ID").then(|| "t".to_owned())), Engine::Claude);
        assert_eq!(engine_for(Some("claude"), claude), Engine::Claude);
        assert_eq!(engine_for(Some("codex"), |_: &str| None), Engine::Codex);
    }
}
