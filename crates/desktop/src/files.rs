//! The files of the squad's projects, for `@` to find by a few letters. Read
//! once per project in the background (a daemon on this Mac only: another
//! machine's disk isn't ours to walk), without what no one mentions — `.git`,
//! `target`, `node_modules`, build output, hidden folders.

use std::path::{Path, PathBuf};

/// More than this many files and the rest of a project isn't listed.
const MOST: usize = 20_000;
const SKIPPED: [&str; 12] = ["target", "node_modules", "dist", "build", "out", "vendor", "Pods", "DerivedData", "__pycache__", "venv", "coverage", "tmp"];

/// One project's files, relative to its folder.
#[derive(Debug, Clone, Default)]
pub struct Index {
    pub project: String,
    pub files: Vec<String>,
}

/// Walk `root` (blocking).
pub fn index(project: &str, root: &Path) -> Index {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if !SKIPPED.contains(&name.as_str()) {
                    stack.push(entry.path());
                }
            } else if kind.is_file()
                && let Ok(rel) = entry.path().strip_prefix(root)
            {
                files.push(rel.to_string_lossy().into_owned());
                if files.len() >= MOST {
                    return Index { project: project.to_owned(), files };
                }
            }
        }
    }
    files.sort();
    Index { project: project.to_owned(), files }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_lists_its_files_without_build_output_or_hidden_folders() {
        let root = std::env::temp_dir().join(format!("equipe-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["src/views", "target/debug", ".git", "node_modules/x"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["src/main.rs", "src/views/chat.rs", "target/debug/app", ".git/HEAD", "node_modules/x/i.js", "README.md", ".env"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let index = index("app", &root);
        assert_eq!(index.files, ["README.md", "src/main.rs", "src/views/chat.rs"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
