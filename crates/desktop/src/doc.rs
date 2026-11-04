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

/// The width the person dragged it to — else a share of the window — kept
/// within what the sidebar leaves: the conversation keeps room to be read.
pub fn kept_width(dragged: Option<f32>, window: f32, sidebar: f32) -> f32 {
    let most = (window - sidebar - CHAT_LEAST).max(MIN_WIDTH);
    dragged.unwrap_or_else(|| width(window)).clamp(MIN_WIDTH, most)
}

/// The conversation's narrowest beside the panel.
pub const CHAT_LEAST: f32 = 420.;

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
    /// A deck shown as a presentation over the window: the slide on show.
    pub presenting: Option<usize>,
    /// Opened afresh rather than read again: a deck starts as a presentation
    /// once its words come.
    pub fresh: bool,
}

impl Doc {
    pub fn new(path: PathBuf, read: u64) -> Self {
        Self { path, text: None, cut: false, scroll: gpui_kit::ScrollHandle::new(), read, folded: false, presenting: None, fresh: true }
    }

    /// Its slides, when it is a deck (`deck.rs`).
    pub fn slides(&self) -> Option<Vec<String>> {
        let text = self.text.as_ref()?.as_ref().ok()?;
        crate::deck::is_deck(&self.path, text).then(|| crate::deck::slides(text))
    }

    /// The slide `at` would show, kept to the deck: none past either end.
    pub fn slide_at(&self, at: usize) -> Option<usize> {
        self.slides().filter(|s| !s.is_empty()).map(|s| at.min(s.len() - 1))
    }

    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Plain text is shown as written, not read as Markdown.
    pub fn markdown(&self) -> Option<String> {
        let text = self.text.as_ref()?.as_ref().ok()?;
        let plain = self.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        Some(if plain { format!("```text\n{text}\n```") } else { crate::deck::body(text).to_owned() })
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

/// Where it is, said short beside its name: its folder from where the
/// conversation works (`outputs/T-287 …`), else from home (`~/…`); nothing
/// when it sits right there. The whole path is a hover away.
pub fn folder_shown(path: &Path, base: &Path, home: Option<&Path>) -> String {
    let Some(folder) = path.parent() else { return String::new() };
    if !base.as_os_str().is_empty()
        && let Ok(rel) = folder.strip_prefix(base)
    {
        return rel.to_string_lossy().into_owned();
    }
    match home.and_then(|h| folder.strip_prefix(h).ok()) {
        Some(rel) => Path::new("~").join(rel).to_string_lossy().into_owned(),
        None => folder.to_string_lossy().into_owned(),
    }
}

/// The tickets a document may be in, as [`elsewhere`] takes them: the one its
/// path names first (`outputs/T-287 …/x.txt`, wherever the person reads it
/// from), then those of the member being read, then every other ticket's
/// worktree — a file handed over before its ticket is merged is still there.
/// Branches are looked through only for the first two: each look is a `git`.
pub type Place = (Option<PathBuf>, Option<String>, Option<PathBuf>);
pub fn places(path: &Path, tickets: &[(u64, bool, Place)]) -> Vec<Place> {
    let path = path.to_string_lossy();
    let named = |id: u64| {
        let tag = format!("T-{id}");
        path.match_indices(&tag).any(|(at, _)| !path[at + tag.len()..].starts_with(|c: char| c.is_ascii_digit()) && !path[..at].ends_with(|c: char| c.is_alphanumeric()))
    };
    let mut ordered: Vec<(u8, &Place)> = tickets
        .iter()
        .filter(|(_, _, (worktree, branch, _))| worktree.is_some() || branch.is_some())
        .map(|(id, ours, place)| (if named(*id) { 0 } else if *ours { 1 } else { 2 }, place))
        .collect();
    ordered.sort_by_key(|(rank, _)| *rank);
    ordered.into_iter().filter_map(|(rank, (worktree, branch, repo))| {
        let branch = branch.clone().filter(|_| rank < 2);
        (worktree.is_some() || branch.is_some()).then(|| (worktree.clone(), branch, repo.clone()))
    }).collect()
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
    fn a_documents_place_is_said_short_from_where_the_conversation_works() {
        let base = Path::new("/Users/a/.equipe/squads/s/home");
        let home = Some(Path::new("/Users/a"));
        assert_eq!(folder_shown(Path::new("/Users/a/.equipe/squads/s/home/outputs/T-287 홍보/A.txt"), base, home), "outputs/T-287 홍보");
        assert_eq!(folder_shown(Path::new("/Users/a/.equipe/squads/s/home/plan.md"), base, home), "");
        assert_eq!(folder_shown(Path::new("/Users/a/.equipe/worktrees/x/notes/a.md"), base, home), "~/.equipe/worktrees/x/notes");
        assert_eq!(folder_shown(Path::new("/opt/x/a.md"), Path::new(""), home), "/opt/x");
    }

    #[test]
    fn a_file_handed_over_on_anyones_ticket_is_found_before_its_ticket_is_merged() {
        let place = |w: &str, b: &str| (Some(PathBuf::from(w)), Some(b.to_owned()), None);
        let tickets = [
            (12, true, place("/w/t12", "t12")),
            (287, false, place("/w/t287", "t287")),
            (28, false, place("/w/t28", "t28")),
            (9, false, (None, None, None)),
        ];
        let got = places(Path::new("/h/outputs/T-287 홍보 글 올리기/A linkedin 붙여넣기.txt"), &tickets);
        assert_eq!(got, [place("/w/t287", "t287"), place("/w/t12", "t12"), (Some(PathBuf::from("/w/t28")), None, None)], "named first, the reader's next, the rest by worktree only");
        let got = places(Path::new("/h/notes/plan.md"), &tickets);
        assert_eq!(got[0], place("/w/t12", "t12"));
        assert_eq!(places(Path::new("/h/T-2870/x.md"), &tickets)[0], place("/w/t12", "t12"), "T-2870 is not T-287");
    }

    #[test]
    fn a_deck_read_in_the_panel_starts_presenting_from_its_first_slide_and_keeps_to_its_ends() {
        let mut doc = Doc::new(PathBuf::from("/s/notes/reports/x/deck.md"), 1);
        assert!(doc.fresh && doc.slides().is_none(), "nothing read yet");
        doc.text = Some(Ok("---\nmarp: true\n---\n# One\n\n---\n\n# Two".into()));
        assert_eq!(doc.slides().map(|s| s.len()), Some(2));
        assert_eq!(doc.slide_at(5), Some(1));
        assert_eq!(doc.markdown().as_deref().map(str::trim_start), Some("# One\n\n---\n\n# Two"), "the front matter isn't the reader's");
        let report = Doc { text: Some(Ok("# Report\n\n---\n\nmore".into())), ..Doc::new(PathBuf::from("/s/report.md"), 2) };
        assert_eq!(report.slides(), None);
    }

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
        assert_eq!(kept_width(None, 1000., 0.), 420.);
        assert_eq!(kept_width(Some(600.), 1400., 0.), 600.);
        assert_eq!(kept_width(Some(100.), 1400., 0.), MIN_WIDTH);
        assert_eq!(kept_width(Some(1300.), 1400., 0.), 980.);
        // Beside the sidebar, the conversation keeps its room: the panel gives way…
        assert_eq!(kept_width(None, 1000., 260.), MIN_WIDTH);
        assert_eq!(kept_width(Some(900.), 1400., 260.), 1400. - 260. - CHAT_LEAST);
        // …down to its narrowest.
        assert_eq!(kept_width(None, 800., 260.), MIN_WIDTH);
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
