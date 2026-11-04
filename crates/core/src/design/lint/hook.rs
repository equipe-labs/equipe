//! The edit-time check: right after a member edits a UI file, the floor
//! rules read in its source — in a second, no browser — and what fails in
//! the part it just wrote goes back to it in the same turn. What was there
//! before its edit isn't put on it.
//!
//! Claude Code runs it after each `Edit`, `MultiEdit` and `Write`, with the
//! call as JSON on stdin (`hooks/hooks.json` in the member's plugin); a
//! failure is said on stderr with exit code 2, which Claude Code hands back
//! to the model. With `ledger`, the edit is also written down in
//! `design/checks/pending.json`, to be checked whole before a hand-in.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Finding, System, Tier, record, scan, source};

/// What one edit asks to have checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub file: PathBuf,
    /// The text it wrote, to find the lines it covers; `None`: the whole file.
    pub written: Vec<String>,
    /// It made the file.
    pub created: bool,
}

/// The edit in a Claude Code hook's input, when it edited a UI file.
pub fn edit_of(input: &Value) -> Option<Edit> {
    let tool = input.get("tool_name")?.as_str()?;
    let args = input.get("tool_input")?;
    let file = PathBuf::from(args.get("file_path").or_else(|| args.get("notebook_path"))?.as_str()?);
    source::Kind::of(&file)?;
    let written: Vec<String> = match tool {
        "Edit" => args.get("new_string").and_then(Value::as_str).map(str::to_owned).into_iter().collect(),
        "MultiEdit" => args.get("edits").and_then(Value::as_array).map(|e| e.iter().filter_map(|x| x.get("new_string")?.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
        "Write" => Vec::new(),
        _ => return None,
    };
    let created = tool == "Write" && input.get("tool_response").and_then(|r| r.get("type")).and_then(Value::as_str) == Some("create");
    Some(Edit { file, written, created })
}

/// The lines of `text` the written pieces cover; `None` when it all counts.
fn lines_written(text: &str, written: &[String]) -> Option<Vec<(u32, u32)>> {
    if written.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for piece in written.iter().filter(|p| !p.trim().is_empty()) {
        if let Some(at) = text.find(piece.as_str()) {
            let first = text[..at].matches('\n').count() as u32 + 1;
            let last = first + piece.matches('\n').count() as u32;
            out.push((first, last));
        }
    }
    Some(out)
}

/// The floor findings in the part of `edit.file` it wrote.
pub fn check(edit: &Edit, system: &System) -> Vec<Finding> {
    let Ok(text) = std::fs::read_to_string(&edit.file) else { return Vec::new() };
    let Some(src) = source::Source::read(&edit.file, text.clone()) else { return Vec::new() };
    let findings = scan::scan(&src, system, super::is_token_file(&edit.file));
    let ranges = lines_written(&text, &edit.written);
    findings
        .into_iter()
        .filter(|f| f.tier == Tier::Floor)
        // A rule about the whole file (motion with no fallback anywhere) counts when the edit touches it.
        .filter(|f| match (&ranges, f.line) {
            (None, _) | (_, None) => true,
            (Some(r), Some(line)) => r.iter().any(|(a, b)| (*a..=*b).contains(&line)),
        })
        .collect()
}

/// What the member is told: each failure with its fix, and that the edit
/// isn't done until they're gone.
pub fn told(file: &Path, findings: &[Finding]) -> String {
    let mut out = format!("equipe design lint: the floor fails in what you just wrote to {} — fix it now (floor rules are never waived):\n", file.display());
    let mut last = "";
    for f in findings {
        if f.rule != last {
            if let Some(r) = super::rules::rule(&f.rule) {
                out.push_str(&format!("- {} — {} Fix: {}\n", r.id, r.checks, r.fix));
            }
            last = &f.rule;
        }
        out.push_str(&format!("    {} {}\n", f.line.map(|l| format!("line {l}:")).unwrap_or_default(), f.evidence));
    }
    out
}

/// Run on a hook's input: the message to hand back, if the floor failed.
/// With `ledger`, the edit is written down to check before a hand-in.
pub fn run(input: &Value, ledger: bool) -> Option<String> {
    let edit = edit_of(input)?;
    let design = super::design_dir_of(&edit.file);
    if ledger && let Some(design) = &design {
        let _ = record::note_edit(design, &edit.file, edit.created);
    }
    let system = design.as_deref().map(System::read).unwrap_or_default();
    let findings = check(&edit, &system);
    (!findings.is_empty()).then(|| told(&edit.file, &findings))
}
