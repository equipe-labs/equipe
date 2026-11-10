//! Every word a person reads, and every word matched in what they or a page
//! write, lives in a crate's `locales/` — none in the code. Korean written in
//! the code would show to someone reading English (or never match Japanese);
//! a language's words go in its own file. And equipe is read and worked on by
//! people everywhere: its comments, docs, scripts and design files are in
//! English too. Korean is one locale among the others, never built in.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{D7B0}'..='\u{D7FF}' | '\u{FFA0}'..='\u{FFDC}')
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

/// The repository's files, by their path from its root with `/`: git's list
/// when there is one (what CI checks out), else every file under the root but
/// build output, dependencies and hidden folders.
fn repository_files(root: &Path) -> Vec<String> {
    let listed = std::process::Command::new("git").args(["ls-files", "-z"]).current_dir(root).output();
    if let Ok(out) = listed
        && out.status.success()
        && !out.stdout.is_empty()
    {
        return String::from_utf8_lossy(&out.stdout).split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect();
    }
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
            let name = entry.file_name().unwrap().to_string_lossy().into_owned();
            if entry.is_dir() {
                if !(name.starts_with('.') && name != ".github") && !["target", "node_modules"].contains(&name.as_str()) {
                    walk(root, &entry, out);
                }
            } else {
                out.push(entry.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// Where another language's words may be: its locale files (and macOS's
/// `<lang>.lproj`), tests and what only tests read — the conformance vectors
/// `cli`'s test-only `remote/vectors.rs` writes — and code vendored as it came.
fn may_hold_any_language(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = &parts[..parts.len() - 1];
    dirs.iter().any(|d| *d == "locales" || *d == "tests" || d.ends_with(".lproj"))
        || parts[0] == "vendor"
        || path.starts_with("docs/remote-vectors/")
}

/// Whether `line` opens an item only tests build: `#[cfg(test)]`, or
/// `#[cfg(all(test, …))]`. (`cfg(any(test, …))` builds without tests too.)
fn test_only_attribute(line: &str) -> bool {
    let code = line.trim_start();
    code.starts_with("#[cfg(test)]") || code.starts_with("#[cfg(all(test")
}

/// The lines of a Rust file that ship, by number from 1: every line but the
/// items under a test-only attribute — an inline `mod tests { … }`, a test
/// helper `fn` — each running from its attribute to the `}` at its own
/// indent (or to its `;`). Also gives the files of test-only modules declared
/// here (`#[cfg(test)] mod tests;`, `#[path = "…"]` honoured), from `path`.
fn shipped_lines(path: &str, text: &str) -> (Vec<(usize, String)>, Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut shipped = Vec::new();
    let mut test_files = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !test_only_attribute(lines[i]) {
            shipped.push((i + 1, lines[i].to_owned()));
            i += 1;
            continue;
        }
        let indent = &lines[i][..lines[i].len() - lines[i].trim_start().len()];
        let mut at = i + 1;
        let mut file_path = None;
        while at < lines.len() && lines[at].trim_start().starts_with("#[") {
            let attr = lines[at].trim();
            if let Some(p) = attr.strip_prefix("#[path = \"").and_then(|r| r.strip_suffix("\"]")) {
                file_path = Some(p.to_owned());
            }
            at += 1;
        }
        let Some(item) = lines.get(at) else { break };
        let code = item.trim();
        let balanced = code.matches('{').count() == code.matches('}').count();
        if code.ends_with(';') && balanced {
            // `mod name;`: its file is a test's too.
            let name = code.trim_end_matches(';').rsplit(' ').next().unwrap_or_default();
            if code.starts_with("mod ") || code.contains(" mod ") {
                let dir = Path::new(path).parent().unwrap_or(Path::new(""));
                let file = Path::new(path).file_stem().unwrap().to_string_lossy();
                let owner = if ["mod", "lib", "main"].contains(&file.as_ref()) { dir.to_path_buf() } else { dir.join(file.as_ref()) };
                let found = match file_path {
                    Some(p) => vec![dir.join(p)],
                    None => vec![owner.join(format!("{name}.rs")), owner.join(name).join("mod.rs")],
                };
                test_files.extend(found.iter().map(|p| p.to_string_lossy().replace('\\', "/")));
            }
            i = at + 1;
            continue;
        }
        if code.ends_with('}') && balanced {
            i = at + 1;
            continue;
        }
        let close = format!("{indent}}}");
        let end = (at + 1..lines.len()).find(|&n| lines[n].starts_with(&close) && !lines[n][close.len()..].starts_with(char::is_whitespace) && lines[n][close.len()..].trim_start_matches([';', ')', ',']).trim().is_empty());
        let Some(end) = end else {
            panic!("{path}:{}: a test-only item with no `}}` at its own indent: write it as rustfmt does", i + 1)
        };
        i = end + 1;
    }
    (shipped, test_files)
}

/// No Hangul in any file that ships or is read to work on equipe — code,
/// comments, prompts and skills members read, scripts, docs, the design
/// folder — outside the places [`may_hold_any_language`] names and the code
/// only tests build.
#[test]
fn no_korean_is_written_into_the_code() {
    let root = root();
    let files = repository_files(&root);
    let mut test_only: HashSet<String> = HashSet::new();
    let mut shipped: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for rel in files.iter().filter(|p| !may_hold_any_language(p)) {
        // Pictures, fonts and other binaries aren't text.
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else { continue };
        if rel.ends_with(".rs") {
            let (lines, tests) = shipped_lines(rel, &text);
            test_only.extend(tests);
            shipped.push((rel.clone(), lines));
        } else {
            shipped.push((rel.clone(), text.lines().enumerate().map(|(n, l)| (n + 1, l.to_owned())).collect()));
        }
    }
    let mut found = Vec::new();
    for (rel, lines) in &shipped {
        if test_only.contains(rel) {
            continue;
        }
        for (n, line) in lines {
            if line.chars().any(hangul) {
                found.push(format!("{rel}:{n}: {}", line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "Korean outside a locale file or a test — a word a person reads goes in the crate's locales/ (every language's file); a word matched in what they write too, \
         matched by a rule that holds for any script; a comment, doc or script says it in English (a UI label as its English words, or its locale key):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_knows_what_only_tests_build() {
    let code = "fn ships() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() {\n        let s = \"}\";\n    }\n}\n\nfn after() {}\n    #[cfg(test)]\n    fn helper() -> u8 {\n        1\n    }\n    fn also() {}\n#[cfg(test)]\n#[path = \"vectors.rs\"]\nmod vectors;\n#[cfg(test)]\npub mod stand_in;\n#[cfg(any(test, feature = \"x\"))]\nfn both() {}\n";
    let (lines, files) = shipped_lines("crates/x/src/remote/core.rs", code);
    let kept: Vec<&str> = lines.iter().map(|(_, l)| l.trim()).filter(|l| !l.is_empty()).collect();
    assert_eq!(kept, ["fn ships() {}", "fn after() {}", "fn also() {}", "#[cfg(any(test, feature = \"x\"))]", "fn both() {}"]);
    assert_eq!(files, ["crates/x/src/remote/vectors.rs", "crates/x/src/remote/core/stand_in.rs", "crates/x/src/remote/core/stand_in/mod.rs"]);
    let (_, files) = shipped_lines("crates/x/src/metrics/mod.rs", "#[cfg(test)]\nmod tests;\n");
    assert_eq!(files, ["crates/x/src/metrics/tests.rs", "crates/x/src/metrics/tests/mod.rs"]);
    assert!(may_hold_any_language("crates/desktop/locales/ko.json") && may_hold_any_language("crates/core/tests/runtime.rs"));
    assert!(may_hold_any_language("crates/desktop/macos/lproj/ko.lproj/InfoPlist.strings") && may_hold_any_language("docs/remote-vectors/messages.json"));
    assert!(!may_hold_any_language("crates/core/skills/design-system/SKILL.md") && !may_hold_any_language("design/guidelines.md"));
    assert!(!may_hold_any_language("crates/desktop/examples/fake_daemon.rs") && !may_hold_any_language("crates/core/src/latest.rs"));
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
