//! `/export`: the conversation on screen as a Markdown file, saved on the Mac
//! the person sits at (never the daemon's) and opened in their editor. The
//! text is the console's (`equipe_core::protocol::transcript::markdown`).

use std::path::{Path, PathBuf};

use equipe_core::protocol::transcript;
use equipe_core::protocol::Entry;

/// `<dir>/<squad>-<member>-<stamp>.md`, with nothing in the name a path would mind.
pub fn file_name(squad: &str, member: &str, stamp: u64) -> String {
    let safe = |s: &str| s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect::<String>();
    format!("{}-{}-{stamp}.md", safe(squad), safe(member))
}

/// Where exports go: Downloads, else the home folder.
pub fn folder(home: &Path) -> PathBuf {
    let downloads = home.join("Downloads");
    if downloads.is_dir() { downloads } else { home.to_owned() }
}

/// Write the conversation; the path it is at.
pub fn save(dir: &Path, squad: &str, member: &str, entries: &[Entry]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(file_name(squad, member, stamp));
    std::fs::write(&path, transcript::markdown(entries, &format!("{member} · {squad}")))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_named_for_the_squad_and_member_without_odd_characters() {
        assert_eq!(file_name("acme", "frontend", 7), "acme-frontend-7.md");
        assert_eq!(file_name("my squad/x", "pm", 1), "my_squad_x-pm-1.md");
    }

    #[test]
    fn a_conversation_is_saved_as_the_consoles_markdown() {
        let dir = std::env::temp_dir().join(format!("equipe-export-{}", std::process::id()));
        let entries = vec![Entry::User { text: "안녕".into() }, Entry::Reply { text: "네.".into() }, Entry::Tool { tool: "Read".into(), target: Some("a.rs".into()), outcome: None, label: None, background: false }];
        let path = save(&dir, "acme", "pm", &entries).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# pm · acme\n\n**You:** 안녕\n\n네.\n\n- read a.rs\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
