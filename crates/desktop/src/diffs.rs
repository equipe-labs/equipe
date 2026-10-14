//! A ticket's diff: where to ask git, how to ask it on this Mac or over SSH
//! (docs/DESKTOP.md, "What the app does on this Mac, and over SSH"), and the
//! unified diff taken apart into files and lines for the viewer. No UI types.

use std::path::PathBuf;

use equipe_core::protocol::{Ticket, TicketId};

use crate::ssh;
use crate::state::Model;
use crate::target::Endpoint;

/// Past this many lines a diff opens with every file folded but the first.
pub const BIG: usize = 5000;

/// Where git is asked: `git -C dir diff from...branch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub dir: PathBuf,
    pub branch: String,
    pub from: &'static str,
}

/// A code ticket's project against `HEAD`; a task's or action's own worktree
/// of the squad's folder against the folder's `main`. None without a branch.
pub fn source(model: &Model, t: &Ticket) -> Option<Source> {
    let branch = t.branch.clone()?;
    if t.kind.in_home() {
        return Some(Source { dir: t.worktree.clone()?, branch, from: "main" });
    }
    let dir = t.project.clone().or_else(|| model.projects.first().cloned())?;
    Some(Source { dir, branch, from: "HEAD" })
}

/// What a fetched stat is good for: while the ticket looks the same.
pub fn fingerprint(t: &Ticket) -> (Option<String>, String, Option<bool>) {
    (t.branch.clone(), t.status.label().to_owned(), t.merged)
}

fn git_script(source: &Source, args: &str) -> String {
    format!("git -C {} {args} {}", ssh::quote(&source.dir.to_string_lossy()), ssh::quote(&format!("{}...{}", source.from, source.branch)))
}

/// Run git for `source`, here or on the daemon's machine. Blocks: call it on a thread.
pub fn run(endpoint: &Endpoint, source: &Source, full: bool) -> Result<String, String> {
    let args = if full { "diff --no-color --no-ext-diff" } else { "diff --no-color --stat=200" };
    match endpoint {
        Endpoint::Local { .. } => {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&source.dir)
                .args(args.split(' '))
                .arg(format!("{}...{}", source.from, source.branch))
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            } else {
                Err(last_line(&String::from_utf8_lossy(&out.stderr)))
            }
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let bytes = runtime.block_on(ssh::Launcher::system(None).run(host, &git_script(source, args))).map_err(|f| f.guide(endpoint).short)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

fn last_line(text: &str) -> String {
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("git이 답하지 않았습니다").trim().to_owned()
}

/// What came back from a fetch thread.
#[derive(Debug)]
pub struct Fetched {
    pub ticket: TicketId,
    pub branch: String,
    pub full: bool,
    pub text: Result<String, String>,
}

/// The `--stat` summary and its files: `changed 3 files · +842 −0`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stat {
    pub files: Vec<String>,
    pub summary: String,
}

pub fn parse_stat(text: &str) -> Stat {
    let files = text.lines().filter(|l| l.contains('|')).map(|l| l.split('|').next().unwrap_or("").trim().to_owned()).collect();
    let summary = text.lines().last().map(str::trim).unwrap_or("").to_owned();
    Stat { files, summary: if summary.contains("changed") { summary } else { String::new() } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hunk,
    Add,
    Del,
    Context,
    /// `\ No newline at end of file`, binary notices.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub lines: Vec<Line>,
}

/// A unified diff taken apart.
pub fn parse(text: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            // `a/x b/x`: the path is after ` b/`.
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, p)| p).to_owned();
            files.push(File { path, ..Default::default() });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some(rest) = raw.strip_prefix("@@") {
            // `@@ -12,5 +14,6 @@ context`
            let (mut o, mut n) = (0, 0);
            for part in rest.split_whitespace().take(2) {
                let num = |s: &str| s.split(',').next().and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
                if let Some(s) = part.strip_prefix('-') {
                    o = num(s);
                } else if let Some(s) = part.strip_prefix('+') {
                    n = num(s);
                }
            }
            (old, new) = (o, n);
            in_hunk = true;
            file.lines.push(Line { kind: Kind::Hunk, old: None, new: None, text: raw.to_owned() });
            continue;
        }
        if !in_hunk {
            // Header lines: only a rename or a binary file are worth a word.
            if raw.starts_with("rename to ") || raw.starts_with("Binary files") || raw.starts_with("new file") || raw.starts_with("deleted file") {
                file.lines.push(Line { kind: Kind::Note, old: None, new: None, text: raw.to_owned() });
            }
            continue;
        }
        match raw.chars().next() {
            Some('+') => {
                file.added += 1;
                file.lines.push(Line { kind: Kind::Add, old: None, new: Some(new), text: raw[1..].to_owned() });
                new += 1;
            }
            Some('-') => {
                file.removed += 1;
                file.lines.push(Line { kind: Kind::Del, old: Some(old), new: None, text: raw[1..].to_owned() });
                old += 1;
            }
            Some('\\') => file.lines.push(Line { kind: Kind::Note, old: None, new: None, text: raw.to_owned() }),
            _ => {
                let text = raw.strip_prefix(' ').unwrap_or(raw).to_owned();
                file.lines.push(Line { kind: Kind::Context, old: Some(old), new: Some(new), text });
                old += 1;
                new += 1;
            }
        }
    }
    files
}

/// The diff as a patch to copy: what git said, as it said it.
pub fn total_lines(files: &[File]) -> usize {
    files.iter().map(|f| f.lines.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,4 @@ fn main
 keep
-gone
+new one
+new two
 tail
\\ No newline at end of file
diff --git a/img.png b/img.png
new file mode 100644
Binary files /dev/null and b/img.png differ
";

    #[test]
    fn a_patch_comes_apart_into_files_and_numbered_lines() {
        let files = parse(PATCH);
        assert_eq!(files.len(), 2);
        let a = &files[0];
        assert_eq!((a.path.as_str(), a.added, a.removed), ("src/a.rs", 2, 1));
        let kinds: Vec<Kind> = a.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, vec![Kind::Hunk, Kind::Context, Kind::Del, Kind::Add, Kind::Add, Kind::Context, Kind::Note]);
        assert_eq!((a.lines[1].old, a.lines[1].new), (Some(1), Some(1)));
        assert_eq!((a.lines[2].old, a.lines[2].new), (Some(2), None));
        assert_eq!((a.lines[3].old, a.lines[3].new), (None, Some(2)));
        assert_eq!((a.lines[5].old, a.lines[5].new), (Some(3), Some(4)));
        assert_eq!(a.lines[3].text, "new one");
        assert_eq!(files[1].path, "img.png");
        assert!(files[1].lines.iter().any(|l| l.text.starts_with("Binary files")));
    }

    #[test]
    fn a_plus_plus_line_inside_a_hunk_is_a_line_not_a_header() {
        let files = parse("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n a\n+++ b\n");
        assert_eq!(files[0].added, 1);
        assert_eq!(files[0].lines.last().unwrap().text, "++ b");
    }

    #[test]
    fn the_stat_gives_files_and_the_summary() {
        let stat = parse_stat(" src/a.rs | 4 ++--\n docs/b.md | 10 ++++++++++\n 2 files changed, 12 insertions(+), 2 deletions(-)\n");
        assert_eq!(stat.files, vec!["src/a.rs", "docs/b.md"]);
        assert_eq!(stat.summary, "2 files changed, 12 insertions(+), 2 deletions(-)");
        assert_eq!(parse_stat(""), Stat::default());
    }

    #[test]
    fn the_ssh_side_command_quotes_what_it_names() {
        let source = Source { dir: PathBuf::from("/Users/o'brien/p"), branch: "equipe/x".into(), from: "HEAD" };
        assert_eq!(git_script(&source, "diff --stat=200"), "git -C '/Users/o'\\''brien/p' diff --stat=200 'HEAD...equipe/x'");
    }

    #[test]
    fn a_code_ticket_is_compared_with_head_in_its_project_and_a_task_with_main_in_its_worktree() {
        let mut model = Model::new("acme");
        model.projects = vec!["/p1".into()];
        let t = |kind: &str| -> Ticket {
            serde_json::from_value(serde_json::json!({"id": 1, "title": "t", "body": "", "status": "review", "kind": kind, "owner": null, "assignee": null, "project": null, "depends_on": [], "branch": "b", "worktree": "/w", "created_by": "pm", "note": null})).unwrap()
        };
        assert_eq!(source(&model, &t("code")), Some(Source { dir: "/p1".into(), branch: "b".into(), from: "HEAD" }));
        assert_eq!(source(&model, &t("task")), Some(Source { dir: "/w".into(), branch: "b".into(), from: "main" }));
        let mut none = t("code");
        none.branch = None;
        assert_eq!(source(&model, &none), None);
    }
}
