//! A design folder's accepted versions: the commits on its repository's main
//! branch that changed it, newest first — each with when it landed, what it
//! says, the ticket it names and which parts it changed (tokens, the logo,
//! the UI map, the guidelines). Read with git when a screen asks
//! (`Command::DesignVersions`), off the core loop, the last [`MAX`] only.
//!
//! Going back to one never rewrites history: [`restore`] words a ticket for
//! the designer, who brings the folder back as it was in its worktree and
//! hands that in for review like any other change.

use std::path::Path;
use std::time::Duration;

use equipe_protocol::design::{AssetKind, DesignPart, DesignVersion};
use equipe_protocol::TicketId;

use crate::i18n::Lang;

/// How many versions are read, newest first.
pub const MAX: usize = 30;
/// The longest the log may take.
const LOG_TIMEOUT: Duration = Duration::from_secs(10);
/// Between commits, and between a commit's fields, in the log's output.
const RECORD: char = '\u{1e}';
const FIELD: char = '\u{1f}';

/// The part of the folder a file in it (`flows/onboarding/login/default.light.390.png`) is.
pub fn part_of(name: &str) -> DesignPart {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        super::folder::TOKENS => DesignPart::Tokens,
        super::folder::GUIDELINES => DesignPart::Guidelines,
        _ if lower.starts_with(&format!("{}/", super::flows::DIR)) => DesignPart::Flows,
        _ if lower.starts_with("assets/") && super::folder::kind_of(&lower) == AssetKind::Logo => DesignPart::Logo,
        _ => DesignPart::Other,
    }
}

/// The ticket a subject names (`designer: T-144 brand`, `equipe: merge
/// T-12 Login`), and the subject without its mark and what leads to it.
pub fn ticket_of(subject: &str) -> (Option<TicketId>, String) {
    let subject = subject.trim();
    let bytes = subject.as_bytes();
    let mut at = 0;
    while let Some(found) = subject[at..].find("T-") {
        let start = at + found;
        let digits = subject[start + 2..].bytes().take_while(u8::is_ascii_digit).count();
        let end = start + 2 + digits;
        let alone_before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let alone_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if digits > 0 && alone_before && alone_after
            && let Ok(id) = subject[start + 2..end].parse::<TicketId>()
        {
            let after = subject[end..].trim_start_matches([' ', ':', '-', '—', '·']).trim();
            let rest = if after.is_empty() { subject[..start].trim().trim_end_matches([' ', ':', '-', '—', '·']).trim() } else { after };
            return (Some(id), rest.to_owned());
        }
        at = start + 2;
    }
    (None, subject.to_owned())
}

/// The versions in what `git log --format=<RS>%H<US>%ct<US>%s --name-only`
/// printed, in its order.
pub fn parse(log: &str) -> Vec<DesignVersion> {
    let mut out = Vec::new();
    for record in log.split(RECORD).filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let head = lines.next().unwrap_or_default();
        let mut fields = head.splitn(3, FIELD);
        let (Some(commit), Some(at), subject) = (fields.next(), fields.next(), fields.next().unwrap_or_default()) else { continue };
        let commit = commit.trim();
        if commit.len() < 7 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let (ticket, subject) = ticket_of(subject);
        let mut parts: Vec<DesignPart> = lines.map(str::trim).filter(|l| !l.is_empty()).map(part_of).collect();
        parts.sort();
        parts.dedup();
        out.push(DesignVersion { commit: commit.to_owned(), at: at.trim().parse().unwrap_or(0), subject, ticket, parts });
    }
    out
}

/// `dir`'s accepted versions: the commits on its repository's main branch
/// that changed it, newest first, at most [`MAX`] — a merge counted once,
/// by what it brought. None when it isn't in a repository with a main
/// branch. Runs git: off the core loop.
pub fn read(dir: &Path) -> Vec<DesignVersion> {
    debug_assert!(!crate::runtime::on_core_loop(), "design versions run git: spawn_blocking them");
    if !dir.is_dir() {
        return Vec::new();
    }
    let Ok(repo) = crate::worktree::repo_root(dir) else { return Vec::new() };
    let Ok(main) = crate::worktree::main_branch(&repo) else { return Vec::new() };
    let mut command = crate::worktree::git_command(dir);
    let max = MAX.to_string();
    let format = format!("--format={RECORD}%H{FIELD}%ct{FIELD}%s");
    let branch = format!("refs/heads/{main}");
    command.args(["-c", "core.quotePath=false", "log", "--first-parent", "-m", "-n", &max, "--relative", "--name-only", "--no-renames", &format, &branch, "--", "."]);
    match crate::worktree::output_within(command, LOG_TIMEOUT) {
        Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// The version `commit` names among `versions` — whole, or its first seven
/// characters or more.
pub fn find<'a>(versions: &'a [DesignVersion], commit: &str) -> Option<&'a DesignVersion> {
    let commit = commit.trim().to_ascii_lowercase();
    if commit.len() < 7 {
        return None;
    }
    versions.iter().find(|v| v.commit.to_ascii_lowercase().starts_with(&commit))
}

/// The ticket that asks the designer for the folder back as it was at
/// `version`: its title and what to do, in the person's language.
pub fn restore(version: &DesignVersion, lang: Lang) -> (String, String) {
    let words = crate::i18n::core(lang);
    let short: String = version.commit.chars().take(7).collect();
    let subject = match version.ticket {
        Some(id) if version.subject.is_empty() => format!("T-{id}"),
        Some(id) => format!("T-{id} {}", version.subject),
        None => version.subject.clone(),
    };
    let title = words.fill("design.restore_title", &[("version", short.clone()), ("subject", subject.clone())]);
    let body = words.fill("design.restore_body", &[("version", short), ("subject", subject), ("commit", version.commit.clone()), ("dir", super::DIR.to_owned())]);
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_file_is_the_part_it_is_in() {
        assert_eq!(part_of("tokens.json"), DesignPart::Tokens);
        assert_eq!(part_of("guidelines.md"), DesignPart::Guidelines);
        assert_eq!(part_of("flows/flows.json"), DesignPart::Flows);
        assert_eq!(part_of("flows/onboarding/login/default.light.390.png"), DesignPart::Flows);
        assert_eq!(part_of("assets/logo/mark.svg"), DesignPart::Logo);
        assert_eq!(part_of("assets/brand/wordmark-dark.svg"), DesignPart::Logo);
        assert_eq!(part_of("assets/icons/bell.svg"), DesignPart::Other);
        assert_eq!(part_of("components/button.png"), DesignPart::Other);
        assert_eq!(part_of("brand.md"), DesignPart::Other);
        assert_eq!(part_of("qa/2026-11-04/report.md"), DesignPart::Other);
    }

    #[test]
    fn a_subject_names_its_ticket_and_reads_without_the_mark() {
        assert_eq!(ticket_of("designer: T-144 브랜드 가이드라인"), (Some(144), "브랜드 가이드라인".into()));
        assert_eq!(ticket_of("equipe: merge T-12 Login screen"), (Some(12), "Login screen".into()));
        assert_eq!(ticket_of("T-7: tokens"), (Some(7), "tokens".into()));
        assert_eq!(ticket_of("Fix the header T-31"), (Some(31), "Fix the header".into()));
        assert_eq!(ticket_of("the person: 로고 교체"), (None, "the person: 로고 교체".into()));
        assert_eq!(ticket_of("GPT-4 notes, AT-3 and T-x"), (None, "GPT-4 notes, AT-3 and T-x".into()), "only a mark of its own");
    }

    #[test]
    fn the_log_reads_as_versions_newest_first_with_what_each_changed() {
        let log = format!(
            "{RECORD}{h1}{FIELD}1793000300{FIELD}equipe: merge T-170 브랜드 리프레시\n\ntokens.json\nflows/onboarding/login/default.light.390.png\nflows/flows.json\nassets/logo/mark.svg\n\
             {RECORD}{h2}{FIELD}1793000200{FIELD}the person: guidelines\n\nguidelines.md\nbrand.md\n\
             {RECORD}{h3}{FIELD}1793000100{FIELD}equipe: the squad's folder: start\n\
             {RECORD}nothex{FIELD}1{FIELD}x\n\ntokens.json\n",
            h1 = "a".repeat(40),
            h2 = "b".repeat(40),
            h3 = "c".repeat(40),
        );
        let versions = parse(&log);
        assert_eq!(versions.len(), 3, "a line that isn't a commit is left out");
        assert_eq!(versions[0], DesignVersion { commit: "a".repeat(40), at: 1_793_000_300, subject: "브랜드 리프레시".into(), ticket: Some(170), parts: vec![DesignPart::Tokens, DesignPart::Logo, DesignPart::Flows] });
        assert_eq!(versions[1].parts, [DesignPart::Guidelines, DesignPart::Other]);
        assert_eq!((versions[1].ticket, versions[1].subject.as_str()), (None, "the person: guidelines"));
        assert!(versions[2].parts.is_empty(), "a subject that has a colon in it reads whole: {:?}", versions[2].subject);
        assert_eq!(versions[2].subject, "equipe: the squad's folder: start");
        assert!(parse("").is_empty());
        assert_eq!(find(&versions, "bbbbbbb").map(|v| v.at), Some(1_793_000_200));
        assert_eq!(find(&versions, &"C".repeat(40)).map(|v| v.at), Some(1_793_000_100));
        assert!(find(&versions, "aaa").is_none(), "too short to name one");
        assert!(find(&versions, "ddddddd").is_none());
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn write(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_versions_are_mains_commits_that_changed_the_folder_a_merge_once() {
        let repo = std::env::temp_dir().join(format!("equipe-design-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        let design = repo.join("design");
        write(design.join("tokens.json"), "{}");
        write(repo.join("README.md"), "app");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "start"]);
        write(repo.join("src.rs"), "fn main() {}");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "code only"]);
        // A ticket's branch: two commits, merged as one.
        git(&repo, &["checkout", "-q", "-b", "equipe/t-170"]);
        write(design.join("flows/app/home/default.light.390.png"), "png");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "wip 1"]);
        write(design.join("assets/logo/mark.svg"), "<svg/>");
        write(design.join("guidelines.md"), "# 가이드");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "wip 2"]);
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--no-ff", "-m", "equipe: merge T-170 브랜드 리프레시", "equipe/t-170"]);
        // Work on another branch, not on main: not a version.
        git(&repo, &["checkout", "-q", "-b", "elsewhere"]);
        write(design.join("tokens.json"), r#"{"x":1}"#);
        git(&repo, &["commit", "-q", "-am", "not accepted"]);
        let versions = read(&design);
        let said: Vec<(&str, Option<TicketId>, &[DesignPart])> = versions.iter().map(|v| (v.subject.as_str(), v.ticket, v.parts.as_slice())).collect();
        assert_eq!(said, [("브랜드 리프레시", Some(170), &[DesignPart::Logo, DesignPart::Flows, DesignPart::Guidelines][..]), ("start", None, &[DesignPart::Tokens][..])]);
        assert!(versions.iter().all(|v| v.commit.len() == 40 && v.at > 0));
        assert!(read(&repo.join("nowhere")).is_empty());
        std::fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn going_back_is_a_ticket_that_names_the_version_and_how() {
        let v = DesignVersion { commit: "0123456789abcdef0123456789abcdef01234567".into(), at: 1, subject: "브랜드 리프레시".into(), ticket: Some(170), parts: vec![] };
        for lang in [Lang::Ko, Lang::En] {
            let (title, body) = restore(&v, lang);
            assert!(title.contains("0123456") && title.contains("T-170"), "{title}");
            assert!(body.contains("git restore --source=0123456789abcdef0123456789abcdef01234567") && body.contains("design"), "{body}");
        }
    }
}
