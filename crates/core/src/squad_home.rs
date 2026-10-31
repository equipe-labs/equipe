//! The squad home: `~/.equipe/squads/<squad>/home`, the squad's own folder.
//!
//! It is where members sit between tickets and where the squad's work that
//! isn't code lives — notes, research, drafts, what tasks hand over. Underneath
//! it is a git repository equipe keeps for the squad, so every change can be
//! told apart and taken back; the person never sees it as one. A change a member
//! makes in conversation is saved as soon as its turn ends, one line saying who
//! and why; whatever the person changed themselves (in Finder, say) is saved
//! under their name before equipe saves or takes back anything else, so nothing
//! they did is lost.

use std::path::{Path, PathBuf};

use crate::journal::squad_dir;

/// Who a change is by when the person made it themselves.
pub const PERSON: &str = "the person";

/// The squad's home folder.
pub fn path(home: &Path, squad: &str) -> PathBuf {
    squad_dir(home, squad).join("home")
}

/// Every other squad's home: what this squad's members may not read.
pub fn others(home: &Path, squad: &str) -> Vec<PathBuf> {
    let Ok(dirs) = std::fs::read_dir(home.join("squads")) else {
        return Vec::new();
    };
    let mut homes: Vec<PathBuf> = dirs
        .flatten()
        .filter(|d| d.file_name() != squad && d.path().join("squad.db").exists())
        .map(|d| d.path().join("home"))
        .collect();
    homes.sort();
    homes
}

/// Make the squad's home if it has none yet. Returns whether it was made.
pub fn ensure(home: &Path, squad: &str, language: Option<&str>) -> Result<bool, String> {
    let dir = path(home, squad);
    if dir.join(".git").exists() {
        quiet(&dir)?;
        return Ok(false);
    }
    for sub in ["notes", "outputs"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|err| err.to_string())?;
        // git keeps no empty folder; these keep the layout for Finder.
        let keep = dir.join(sub).join(".gitkeep");
        if !keep.exists() {
            std::fs::write(keep, "").map_err(|err| err.to_string())?;
        }
    }
    let readme = dir.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, readme_text(squad, language)).map_err(|err| err.to_string())?;
    }
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, ".DS_Store\n*.tmp\n").map_err(|err| err.to_string())?;
    }
    git(&dir, &["init", "-q", "-b", "main"])?;
    // Its own name, so it works without the person's git setup and its history never carries theirs.
    git(&dir, &["config", "user.name", "equipe"])?;
    git(&dir, &["config", "user.email", "equipe@localhost"])?;
    quiet(&dir)?;
    git(&dir, &["add", "-A"])?;
    git(&dir, &["commit", "-q", "-m", "equipe: the squad's folder"])?;
    Ok(true)
}

/// No signing and no hooks, in the home's own settings too: tickets' worktrees of
/// it commit and merge with plain git, which reads them.
fn quiet(dir: &Path) -> Result<(), String> {
    git(dir, &["config", "commit.gpgsign", "false"])?;
    git(dir, &["config", "core.hooksPath", "/dev/null"]).map(|_| ())
}

fn readme_text(squad: &str, language: Option<&str>) -> String {
    let words = crate::i18n::core(named(language));
    format!(
        "# {}\n\n{}\n\n- `notes/` — {}\n- `outputs/` — {}\n\n{}\n",
        words.fill("squad_home.readme_title", &[("squad", squad.to_owned())]),
        words.get("squad_home.readme_intro"),
        words.get("squad_home.readme_notes"),
        words.get("squad_home.readme_outputs"),
        words.get("squad_home.readme_edit"),
    )
}

/// The language a squad's `language` (its English name: `Korean`) names; English when none.
pub(crate) fn named(language: Option<&str>) -> crate::i18n::Lang {
    crate::i18n::Lang::ALL.into_iter().find(|l| Some(l.english_name()) == language).unwrap_or(crate::i18n::Lang::En)
}

/// One saved change to the home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    /// Who made it: a member's name, or [`PERSON`].
    pub by: String,
    /// Why, in a line.
    pub why: String,
    /// Unix seconds.
    pub at: i64,
    /// What it did to which file: `("saved", "notes/a.md")`, `("updated", …)`, `("removed", …)`.
    pub files: Vec<(&'static str, String)>,
}

impl Change {
    /// "saved notes/a.md, updated outputs/b.md" — no git words.
    pub fn what(&self) -> String {
        let mut parts: Vec<String> = self.files.iter().take(3).map(|(verb, file)| format!("{verb} {file}")).collect();
        if self.files.len() > 3 {
            parts.push(format!("{} more", self.files.len() - 3));
        }
        if parts.is_empty() { "nothing".to_owned() } else { parts.join(", ") }
    }

    /// [`Change::what`] in `lang`, for the person.
    pub fn said(&self, lang: crate::i18n::Lang) -> String {
        let words = crate::i18n::core(lang);
        let mut parts: Vec<String> = self
            .files
            .iter()
            .take(3)
            .map(|(verb, file)| {
                let key = match *verb {
                    "saved" => "change.saved",
                    "removed" => "change.removed",
                    _ => "change.updated",
                };
                words.fill(key, &[("file", file.clone())])
            })
            .collect();
        if self.files.len() > 3 {
            parts.push(words.fill("change.more", &[("count", (self.files.len() - 3).to_string())]));
        }
        if parts.is_empty() { words.get("change.nothing").to_owned() } else { parts.join(words.get("list.separator")) }
    }
}

/// Save whatever changed in the home, by `by` for `why`. `None` when nothing had.
pub fn save(dir: &Path, by: &str, why: &str) -> Result<Option<Change>, String> {
    save_in(dir, by, why, &[])
}

/// Save whatever changed under `paths` only (all of the home when empty).
pub fn save_in(dir: &Path, by: &str, why: &str, paths: &[&str]) -> Result<Option<Change>, String> {
    let with_paths = |args: &[&'static str]| -> Vec<&str> { args.iter().copied().chain(["--"]).chain(paths.iter().copied()).collect() };
    if !dir.join(".git").exists() || git(dir, &with_paths(&["status", "--porcelain"]))?.trim().is_empty() {
        return Ok(None);
    }
    git(dir, &with_paths(&["add", "-A"]))?;
    let message = format!("{by}: {}", one_line(why));
    let mut commit = vec!["commit", "-q", "-m", &message];
    commit.extend(with_paths(&[]));
    git(dir, &commit)?;
    Ok(changes(dir, 1)?.into_iter().next())
}

/// Save what the person changed themselves, before equipe changes anything.
pub fn save_persons(dir: &Path) -> Result<Option<Change>, String> {
    save(dir, PERSON, "changed in the squad's folder")
}

/// The latest `n` changes, newest first. The folder's making isn't one.
pub fn changes(dir: &Path, n: usize) -> Result<Vec<Change>, String> {
    changes_in(dir, n, &[])
}

/// The latest `n` changes that touched `paths` (all of the home when empty).
pub fn changes_in(dir: &Path, n: usize, paths: &[&str]) -> Result<Vec<Change>, String> {
    let count = format!("-{n}");
    let mut args = vec!["log", &count, "--name-status", "--no-renames", "--format=%x1e%H%x1f%ct%x1f%s", "--"];
    args.extend(paths);
    let log = git(dir, &args)?;
    let mut out = Vec::new();
    for record in log.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let mut head = lines.next().unwrap_or_default().split('\u{1f}');
        let (id, at, subject) = (head.next().unwrap_or_default(), head.next().unwrap_or_default(), head.next().unwrap_or_default());
        let Some((by, why)) = subject.split_once(": ") else {
            continue;
        };
        if by == "equipe" {
            continue;
        }
        let files = lines
            .filter_map(|line| {
                let (status, file) = line.split_once('\t')?;
                // equipe keeps the memory's index itself; it is never news.
                if file.ends_with(".gitkeep") || file == crate::squad_memory::INDEX_PATH {
                    return None;
                }
                let verb = match status {
                    "A" => "saved",
                    "D" => "removed",
                    _ => "updated",
                };
                Some((verb, file.to_owned()))
            })
            .collect();
        out.push(Change { id: id.to_owned(), by: by.to_owned(), why: why.to_owned(), at: at.parse().unwrap_or(0), files });
    }
    Ok(out)
}

/// Take back the `nth` latest change (1 = the latest), as a change of its own by
/// the person — so an undo can be undone too. The person's own unsaved edits are
/// saved first. A change that later ones built on is left as it is.
pub fn undo(dir: &Path, nth: usize) -> Result<Change, String> {
    save_persons(dir)?;
    let recent = changes(dir, nth.max(1))?;
    let Some(target) = recent.get(nth.max(1) - 1).cloned() else {
        return Err("There is no such change to take back.".into());
    };
    if git(dir, &["revert", "--no-commit", &target.id]).is_err() {
        let _ = git(dir, &["revert", "--abort"]);
        let _ = git(dir, &["reset", "-q", "--hard", "HEAD"]);
        return Err(format!("Later changes built on “{}”, so it can't be taken back on its own.", target.why));
    }
    let why = format!("took back {}'s “{}”", target.by, target.why);
    git(dir, &["commit", "-q", "--allow-empty", "-m", &format!("{PERSON}: {}", one_line(&why))])?;
    Ok(changes(dir, 1)?.into_iter().next().expect("just saved"))
}

/// Which squad home, if any, `dir` is in: `~/.equipe/squads/<squad>/home/…`.
pub fn squad_of(home: &Path, dir: &Path) -> Option<String> {
    let rest = dir.strip_prefix(home.join("squads")).ok()?;
    let mut parts = rest.components();
    let squad = parts.next()?.as_os_str().to_string_lossy().into_owned();
    (parts.next()?.as_os_str() == "home").then_some(squad)
}

/// The links cited in `files` (paths under `dir`), first seen first: where a
/// task's findings say they came from, for the person to check.
pub fn cited_sources(dir: &Path, files: &[String]) -> Vec<String> {
    const MAX: usize = 12;
    let mut found: Vec<String> = Vec::new();
    for file in files {
        let Ok(bytes) = std::fs::read(dir.join(file)) else { continue };
        // Text only, and not a whole dump.
        let Ok(text) = std::str::from_utf8(&bytes[..bytes.len().min(1 << 20)]) else { continue };
        for start in text.match_indices("http").map(|(i, _)| i) {
            let rest = &text[start..];
            if !(rest.starts_with("https://") || rest.starts_with("http://")) {
                continue;
            }
            let end = rest.find(|c: char| c.is_whitespace() || "<>()[]\"'`".contains(c)).unwrap_or(rest.len());
            let link = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if link.len() > "https://".len() && !found.iter().any(|f| f == link) {
                found.push(link.to_owned());
                if found.len() == MAX {
                    return found;
                }
            }
        }
    }
    found
}

fn one_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no reason given");
    if line.chars().count() > 80 { format!("{}…", line.chars().take(79).collect::<String>()) } else { line.to_owned() }
}

/// git in the home, independent of the person's own git setup: no signing, no hooks.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = crate::worktree::git_command(dir);
    command
        .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "-c", "core.quotePath=false", "-c", "user.name=equipe", "-c", "user.email=equipe@localhost"])
        .args(args);
    let out = crate::worktree::output_within(command, crate::worktree::GIT_TIMEOUT)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_told_in_the_persons_language() {
        use crate::i18n::Lang;
        let files = |n: usize| (0..n).map(|i| (["saved", "updated", "removed"][i % 3], format!("notes/{i}.md"))).collect();
        let change = |n| Change { id: "x".into(), by: "pm".into(), why: "w".into(), at: 0, files: files(n) };
        assert_eq!(change(4).said(Lang::En), change(4).what(), "English says what it always said");
        assert_eq!(change(4).said(Lang::Ko), "notes/0.md 저장, notes/1.md 수정, notes/2.md 삭제, 외 1개");
        assert_eq!(change(5).said(Lang::Ja), "notes/0.md を保存、notes/1.md を更新、notes/2.md を削除、ほか 2 件");
        assert_eq!(change(0).said(Lang::De), "nichts");
    }

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn home() -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-home-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_home_is_made_once_with_its_layout() {
        let home = home();
        assert!(ensure(&home, "acme", Some("Korean")).unwrap());
        assert!(!ensure(&home, "acme", None).unwrap(), "made once");
        let dir = path(&home, "acme");
        assert!(dir.join("notes").is_dir() && dir.join("outputs").is_dir());
        assert!(std::fs::read_to_string(dir.join("README.md")).unwrap().contains("스쿼드 폴더"));
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), ".DS_Store\n*.tmp\n");
        assert_eq!(git(&dir, &["branch", "--show-current"]).unwrap().trim(), "main");
        assert_eq!(git(&dir, &["log", "-1", "--format=%an"]).unwrap().trim(), "equipe");
        assert!(changes(&dir, 10).unwrap().is_empty(), "making the folder isn't a change to show");
        assert_eq!(squad_of(&home, &dir.join("notes")).as_deref(), Some("acme"));
        assert_eq!(squad_of(&home, &home.join("squads/acme/members")), None);
    }

    #[test]
    fn changes_are_saved_with_who_and_why_and_can_be_taken_back() {
        let home = home();
        ensure(&home, "acme", None).unwrap();
        let dir = path(&home, "acme");
        assert_eq!(save(&dir, "pm", "nothing").unwrap(), None, "nothing changed, nothing saved");

        std::fs::write(dir.join("notes/세금-마감.md"), "5월 31일\n").unwrap();
        let saved = save(&dir, "pm", "세금 마감일 메모해줘\nand more").unwrap().unwrap();
        assert_eq!((saved.by.as_str(), saved.why.as_str()), ("pm", "세금 마감일 메모해줘"));
        assert_eq!(saved.what(), "saved notes/세금-마감.md");

        // The person edits in Finder; the undo of pm's note keeps their edit.
        std::fs::write(dir.join("notes/mine.md"), "mine\n").unwrap();
        assert!(undo(&dir, 9).unwrap_err().contains("no such change"));
        // Their edit was saved first, so pm's note is now the second latest.
        let undone = undo(&dir, 2).unwrap();
        assert_eq!(undone.by, PERSON);
        assert_eq!(undone.what(), "removed notes/세금-마감.md");
        assert!(dir.join("notes/mine.md").exists(), "the person's own change is kept");
        let list = changes(&dir, 10).unwrap();
        assert_eq!(list.iter().map(|c| c.by.as_str()).collect::<Vec<_>>(), [PERSON, PERSON, "pm"]);

        // An undo is a change too: undoing it brings the note back.
        undo(&dir, 1).unwrap();
        assert!(dir.join("notes/세금-마감.md").exists());
    }

    #[test]
    fn a_change_others_built_on_is_not_taken_back_alone() {
        let home = home();
        ensure(&home, "acme", None).unwrap();
        let dir = path(&home, "acme");
        std::fs::write(dir.join("notes/a.md"), "one\n").unwrap();
        save(&dir, "pm", "first").unwrap();
        std::fs::write(dir.join("notes/a.md"), "two\n").unwrap();
        save(&dir, "backend", "second").unwrap();
        let err = undo(&dir, 2).unwrap_err();
        assert!(err.contains("first"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("notes/a.md")).unwrap(), "two\n", "left as it was");
        assert!(git(&dir, &["status", "--porcelain"]).unwrap().trim().is_empty());
    }

    #[test]
    fn other_squads_homes_are_listed_for_fencing_off() {
        let home = home();
        for squad in ["acme", "beta"] {
            std::fs::create_dir_all(squad_dir(&home, squad)).unwrap();
            std::fs::write(squad_dir(&home, squad).join("squad.db"), "").unwrap();
        }
        assert_eq!(others(&home, "acme"), [path(&home, "beta")]);
    }

    #[test]
    fn the_links_a_tasks_files_cite_are_its_sources() {
        let dir = home();
        std::fs::write(
            dir.join("deadlines.md"),
            "- 5월 31일 종합소득세 (출처: https://www.nts.go.kr/a.do?x=1).\n- [부가세](https://www.nts.go.kr/b), 확인 안 됨\n같은 곳 https://www.nts.go.kr/a.do?x=1\n",
        )
        .unwrap();
        std::fs::write(dir.join("chart.png"), [0xff, 0xfe, 0x00]).unwrap();
        let files = ["deadlines.md".to_owned(), "chart.png".to_owned(), "gone.md".to_owned()];
        assert_eq!(cited_sources(&dir, &files), ["https://www.nts.go.kr/a.do?x=1", "https://www.nts.go.kr/b"]);
        assert!(cited_sources(&dir, &files[1..]).is_empty());
    }
}
