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
}

impl Doc {
    pub fn new(path: PathBuf, read: u64) -> Self {
        Self { path, text: None, cut: false, scroll: gpui_kit::ScrollHandle::new(), read }
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

/// Read `path` where the daemon is (blocking): its words, and whether they were cut.
pub fn read(endpoint: &Endpoint, path: &Path) -> Result<(String, bool), String> {
    let bytes = match endpoint {
        Endpoint::Local { .. } => {
            use std::io::Read as _;
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(MOST as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            bytes
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let script = format!("head -c {} -- {}", MOST + 1, ssh::quote(&path.to_string_lossy()));
            runtime.block_on(ssh::Launcher::system(None).run(host, &script)).map_err(|f| f.guide(endpoint).short)?
        }
    };
    Ok(words(bytes))
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
    fn a_file_here_is_read_whole() {
        let path = std::env::temp_dir().join(format!("equipe-doc-{}.md", std::process::id()));
        std::fs::write(&path, "# 설계\n\n본문").unwrap();
        let endpoint = Endpoint::Local { socket: "/nonexistent".into() };
        assert_eq!(read(&endpoint, &path).unwrap(), ("# 설계\n\n본문".into(), false));
        assert!(read(&endpoint, Path::new("/nonexistent/x.md")).is_err());
        let _ = std::fs::remove_file(path);
    }
}
