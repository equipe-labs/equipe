//! A document a member wrote — a design, a plan, notes — read beside the
//! conversation: a Markdown file named in a reply or a step opens in the
//! panel at the right, drawn as the conversation draws Markdown. The file is
//! on the daemon's machine: read here when that is this Mac, else over the
//! same SSH connection. No UI types here.

use std::path::{Path, PathBuf};

use crate::ssh;
use crate::target::Endpoint;

/// More than this is cut, and the panel says so.
pub const MOST: usize = 1 << 20;

/// What opens in the panel rather than Quick Look: Markdown and plain text.
pub fn is_doc(path: &str) -> bool {
    let name = Path::new(crate::render::links::path_and_line(path).0).file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    [".md", ".markdown", ".mdx", ".txt"].iter().any(|ext| name.ends_with(ext) && name.len() > ext.len())
}

/// How wide the panel is: a share of the window, within reason.
pub fn width(window: f32) -> f32 {
    (window * 0.42).clamp(380., 820.)
}

/// The width the person dragged it to, kept within the window: the
/// conversation keeps room to be read.
pub fn kept_width(dragged: Option<f32>, window: f32) -> f32 {
    let most = (window - 420.).max(MIN_WIDTH);
    dragged.map_or_else(|| width(window), |w| w.clamp(MIN_WIDTH, most))
}

/// The panel's narrowest.
pub const MIN_WIDTH: f32 = 320.;
/// Folded: a rail with its name, to open it again.
pub const RAIL: f32 = 40.;

/// The document in the panel.
pub struct Doc {
    pub path: PathBuf,
    /// `None` while it is read; then its words, or why not.
    pub text: Option<Result<String, String>>,
    /// More than [`MOST`] was there: the rest isn't shown.
    pub cut: bool,
    pub scroll: gpui_kit::ScrollHandle,
    /// Which read this is: an older one coming back late is dropped.
    pub read: u64,
    /// Folded to a rail at the window's edge.
    pub folded: bool,
}

impl Doc {
    pub fn new(path: PathBuf, read: u64) -> Self {
        Self { path, text: None, cut: false, scroll: gpui_kit::ScrollHandle::new(), read, folded: false }
    }

    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Plain text is shown as written, not read as Markdown.
    pub fn markdown(&self) -> Option<String> {
        let text = self.text.as_ref()?.as_ref().ok()?;
        let plain = self.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        Some(if plain { format!("```text\n{text}\n```") } else { text.clone() })
    }
}

/// Where else a document named in a conversation may be: a member writes in
/// its ticket's worktree, and a ticket handed in but not merged keeps it on
/// its branch only — the path as written, read against the folder the
/// conversation is in, isn't there yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Elsewhere {
    /// The same file in each worktree the member works in.
    pub paths: Vec<PathBuf>,
    /// `git show <branch>:<path>` in a repository.
    pub branches: Vec<(PathBuf, String, String)>,
}

/// The places to look for `path` beyond itself: `base` is the folder the
/// conversation is in; `tickets` (worktree, branch, repository) the member's.
pub fn elsewhere(path: &Path, base: &Path, tickets: &[(Option<PathBuf>, Option<String>, Option<PathBuf>)]) -> Elsewhere {
    let mut out = Elsewhere::default();
    let Ok(rel) = path.strip_prefix(base) else { return out };
    for (worktree, branch, repo) in tickets {
        if let Some(w) = worktree {
            out.paths.push(w.join(rel));
        }
        if let Some(b) = branch {
            out.branches.push((repo.clone().unwrap_or_else(|| base.to_owned()), b.clone(), rel.to_string_lossy().into_owned()));
        }
    }
    out
}

/// Read `path` where the daemon is (blocking), else where it went
/// (`elsewhere`): its words, and whether they were cut.
pub fn read(endpoint: &Endpoint, path: &Path, also: &Elsewhere) -> Result<(String, bool), String> {
    let bytes = match endpoint {
        Endpoint::Local { .. } => {
            use std::io::Read as _;
            let found = std::iter::once(path.to_owned()).chain(also.paths.iter().cloned()).find(|p| p.is_file());
            match found {
                Some(found) => {
                    let file = std::fs::File::open(&found).map_err(|e| e.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(MOST as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    bytes
                }
                None => also
                    .branches
                    .iter()
                    .find_map(|(repo, branch, rel)| {
                        let out = std::process::Command::new("git").arg("-C").arg(repo).args(["show", &format!("{branch}:{rel}")]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
                        out.status.success().then_some(out.stdout)
                    })
                    .ok_or_else(|| format!("{}: no such file", path.display()))?,
            }
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let bytes = runtime.block_on(ssh::Launcher::system(None).run(host, &script(path, also))).map_err(|f| f.detail().map_or_else(|| f.guide(endpoint).short, str::to_owned))?;
            if bytes.is_empty() {
                // Said nothing: none of the places had it (an empty file reads as empty too).
                return Ok((String::new(), false));
            }
            bytes
        }
    };
    Ok(words(bytes))
}

/// The far machine's way through the places: the first file there is read;
/// else the first branch that has it; else it says it found none.
fn script(path: &Path, also: &Elsewhere) -> String {
    let most = MOST + 1;
    let mut s = String::new();
    for p in std::iter::once(path.to_owned()).chain(also.paths.iter().cloned()) {
        let q = ssh::quote(&p.to_string_lossy());
        s.push_str(&format!("if [ -f {q} ]; then exec head -c {most} -- {q}; fi\n"));
    }
    for (repo, branch, rel) in &also.branches {
        let spec = ssh::quote(&format!("{branch}:{rel}"));
        s.push_str(&format!("if git -C {} cat-file -e {spec} 2>/dev/null; then git -C {} show {spec} | head -c {most}; exit 0; fi\n", ssh::quote(&repo.to_string_lossy()), ssh::quote(&repo.to_string_lossy())));
    }
    s.push_str(&format!("echo {} >&2; exit 1\n", ssh::quote(&format!("{}: no such file", path.display()))));
    s
}

/// Bytes as text, cut at [`MOST`] on a character's edge.
fn words(mut bytes: Vec<u8>) -> (String, bool) {
    let cut = bytes.len() > MOST;
    bytes.truncate(MOST);
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        // Cut inside a character, or not quite UTF-8: what reads, reads.
        Err(e) => String::from_utf8_lossy(e.as_bytes()).trim_end_matches('\u{FFFD}').to_owned(),
    };
    (text, cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_and_text_open_in_the_panel() {
        assert!(is_doc("outputs/T-4 데이터 설계/design.md"));
        assert!(is_doc("/x/README.MD"));
        assert!(is_doc("notes.txt:12"));
        assert!(is_doc("docs/plan.markdown"));
        assert!(!is_doc("src/main.rs"));
        assert!(!is_doc(".md"));
        assert!(!is_doc("design.md.png"));
    }

    #[test]
    fn the_panel_takes_a_share_of_the_window_within_reason() {
        assert_eq!(width(1000.), 420.);
        assert_eq!(width(600.), 380.);
        assert_eq!(width(3000.), 820.);
    }

    #[test]
    fn a_dragged_width_leaves_the_conversation_room() {
        assert_eq!(kept_width(None, 1000.), 420.);
        assert_eq!(kept_width(Some(600.), 1400.), 600.);
        assert_eq!(kept_width(Some(100.), 1400.), MIN_WIDTH);
        assert_eq!(kept_width(Some(1300.), 1400.), 980.);
    }

    #[test]
    fn a_long_file_is_cut_on_a_characters_edge() {
        let (text, cut) = words("가".repeat(MOST).into_bytes());
        assert!(cut && text.len() <= MOST && text.chars().all(|c| c == '가'));
        assert_eq!(words(b"# hi\n".to_vec()), ("# hi\n".into(), false));
    }

    #[test]
    fn plain_text_is_shown_as_written() {
        let mut doc = Doc::new("/a/notes.txt".into(), 1);
        doc.text = Some(Ok("*not emphasis*".into()));
        assert_eq!(doc.markdown().unwrap(), "```text\n*not emphasis*\n```");
        doc.path = "/a/plan.md".into();
        assert_eq!(doc.markdown().unwrap(), "*not emphasis*");
    }

    #[test]
    fn a_document_in_a_tickets_worktree_or_branch_is_looked_for_there() {
        let base = Path::new("/h/squads/acme/home");
        let tickets = vec![(Some(PathBuf::from("/h/worktrees/acme.home/t10-ab")), Some("equipe/backend-t10".to_owned()), None)];
        let also = elsewhere(&base.join("outputs/T-10 회사 계정/log.md"), base, &tickets);
        assert_eq!(also.paths, vec![PathBuf::from("/h/worktrees/acme.home/t10-ab/outputs/T-10 회사 계정/log.md")]);
        assert_eq!(also.branches, vec![(base.to_owned(), "equipe/backend-t10".to_owned(), "outputs/T-10 회사 계정/log.md".to_owned())]);
        assert_eq!(elsewhere(Path::new("/elsewhere/x.md"), base, &tickets), Elsewhere::default(), "not under the folder: nowhere else");
        let script = script(Path::new("/a b/x.md"), &also);
        assert!(script.starts_with("if [ -f '/a b/x.md' ]") && script.contains("git -C '/h/squads/acme/home' cat-file -e 'equipe/backend-t10:outputs/T-10 회사 계정/log.md'"), "{script}");
    }

    #[test]
    fn a_file_here_is_read_whole() {
        let path = std::env::temp_dir().join(format!("equipe-doc-{}.md", std::process::id()));
        std::fs::write(&path, "# 설계\n\n본문").unwrap();
        let endpoint = Endpoint::Local { socket: "/nonexistent".into() };
        assert_eq!(read(&endpoint, &path, &Elsewhere::default()).unwrap(), ("# 설계\n\n본문".into(), false));
        assert!(read(&endpoint, Path::new("/nonexistent/x.md"), &Elsewhere::default()).is_err());
        // Not where it was named, but in the worktree it was written in.
        let also = Elsewhere { paths: vec![path.clone()], branches: vec![] };
        assert_eq!(read(&endpoint, Path::new("/nonexistent/x.md"), &also).unwrap().0, "# 설계\n\n본문");
        let _ = std::fs::remove_file(path);
    }
}
