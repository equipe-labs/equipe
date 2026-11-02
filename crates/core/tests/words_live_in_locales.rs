//! Every word a person reads, and every word matched in what they or a page
//! write, lives in a crate's `locales/` — none in the code. Korean written in
//! the code would show to someone reading English (or never match Japanese);
//! a language's words go in its own file.

use std::path::{Path, PathBuf};

/// Code that only tests build: test data may be in any language.
const TEST_ONLY: &[&str] = &["crates/cli/src/remote/vectors.rs"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
        if entry.is_dir() {
            rust_files(&entry, out);
        } else if entry.extension().is_some_and(|e| e == "rs") {
            out.push(entry);
        }
    }
}

fn hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
}

#[test]
fn no_korean_is_written_into_the_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let mut files = Vec::new();
    for krate in std::fs::read_dir(root.join("crates")).unwrap().map(|e| e.unwrap().path()) {
        if krate.join("src").is_dir() {
            rust_files(&krate.join("src"), &mut files);
        }
    }
    let mut found = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if TEST_ONLY.contains(&rel.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut test_next = false;
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            // A test module runs to the file's end, as they are written here.
            if code.starts_with("#[cfg(test)]") {
                test_next = true;
                continue;
            }
            if test_next && (code.starts_with("mod ") || code.starts_with("pub mod ") || code.starts_with("pub(crate) mod ")) && code.ends_with('{') {
                break;
            }
            test_next = false;
            if code.starts_with("//") {
                continue;
            }
            // What follows a `//` outside a string is a comment.
            let mut in_string = false;
            let mut prev = ' ';
            let mut cut = code.len();
            for (i, c) in code.char_indices() {
                if c == '"' && prev != '\\' {
                    in_string = !in_string;
                }
                if !in_string && c == '/' && prev == '/' {
                    cut = i - 1;
                    break;
                }
                prev = c;
            }
            if code[..cut].chars().any(hangul) {
                found.push(format!("{rel}:{}: {}", n + 1, code));
            }
        }
    }
    assert!(found.is_empty(), "Korean in the code — move it to the crate's locales/:\n{}", found.join("\n"));
}

/// Files built into equipe and handed to people as they are (a design
/// folder's `brand.md`, a browser script): one text in every language, so
/// none of it in Korean.
#[test]
fn no_korean_is_built_in_with_the_assets() {
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
            if entry.is_dir() {
                files(&entry, out);
            } else {
                out.push(entry);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let mut all = Vec::new();
    for krate in std::fs::read_dir(root.join("crates")).unwrap().map(|e| e.unwrap().path()) {
        if krate.join("assets").is_dir() {
            files(&krate.join("assets"), &mut all);
        }
    }
    let mut found = Vec::new();
    for path in all {
        // Pictures and fonts aren't text.
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        for (n, line) in text.lines().enumerate() {
            if line.chars().any(hangul) {
                found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "Korean in a built-in file — write it in English, or put the words in locales/:\n{}", found.join("\n"));
}
