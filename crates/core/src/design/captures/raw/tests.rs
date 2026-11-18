use super::*;
use crate::worktree::git;

fn scratch(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-raw-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn commit_all(dir: &Path, message: &str, force: &[&str]) {
    git(dir, &["add", "-A"]).unwrap();
    for path in force {
        git(dir, &["add", "-f", "--", path]).unwrap();
    }
    git(dir, &["commit", "-q", "-m", message]).unwrap();
}

fn tracked(dir: &Path, rev: &str) -> Vec<String> {
    let out = worktree::git_bytes(dir, &["-c", "core.quotePath=false", "ls-tree", "-r", "-z", "--name-only", rev], None, None).unwrap();
    out.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect()
}

/// Whether git ignores the file `rel` in `dir` (made, if it isn't there).
fn ignored(dir: &Path, rel: &str) -> bool {
    if !dir.join(rel).exists() {
        write(&dir.join(rel), b"x");
    }
    git(dir, &["check-ignore", "-q", "--no-index", "--", rel]).is_ok()
}

/// A squad's folder the way an older equipe kept it: one hand-in with its
/// summary, a probe's dumps, logs and scripts, and 25 notes, all tracked.
fn squad_folder() -> PathBuf {
    let dir = scratch("equipe").join("squads/acme/home");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]).unwrap();
    git(&dir, &["config", "user.name", "equipe"]).unwrap();
    git(&dir, &["config", "user.email", "equipe@localhost"]).unwrap();
    write(&dir.join("notes/research.md"), b"# Research");
    write(&dir.join("outputs/T-1 probe/README.md"), b"# Probe\n\nSee raw/ for the matrix.");
    write(&dir.join("outputs/T-1 probe/probe.mjs"), b"console.log(1)");
    write(&dir.join("outputs/T-1 probe/data/390-dark.json"), b"{}");
    write(&dir.join("outputs/T-1 probe/data/run.log"), b"ok");
    write(&dir.join("outputs/T-1 probe/dom/page.html"), b"<html></html>");
    for n in 0..25 {
        write(&dir.join(format!("outputs/T-1 probe/notes/{n:02}.md")), format!("note {n}").as_bytes());
    }
    write(&dir.join("outputs/T-2 brief/brief.md"), b"# Brief");
    commit_all(&dir, "the squad's folder", &[]);
    dir
}

#[test]
fn machine_output_is_a_raw_folder_or_a_machine_kind_under_outputs() {
    for yes in ["outputs/T-1/probe.json", "outputs/T-1/data/a.JSONL", "outputs/T-1/run.log", "outputs/T-1/probe.mjs", "outputs/T-1/dom/page.html", "outputs/T-1/raw/notes.md", "outputs/T-1/a/node_modules/x/README.md", "outputs/loose.json", "outputs/T-1/.a.dark.390.png.audit.json"] {
        assert!(is_machine(yes), "{yes}");
    }
    for no in ["outputs/T-1/README.md", "outputs/T-1/table.csv", "outputs/T-1/raw-notes.md", "outputs/.captures.json", "outputs/T-1/.gitignore", "outputs/T-1/.gitkeep", "notes/a.json", "outputs/../a.json", "src/outputs/a.json", "outputs/T-1/.json"] {
        assert!(!is_machine(no), "{no}");
    }
    assert!(readable("outputs/T-1/summary.txt") && !readable("outputs/T-1/shot.png") && !readable("outputs/T-1/deck.pptx") && !readable("outputs/T-1/raw/a.md"));
    assert!(OUTPUTS.keeps("outputs/T-1/chart.svg") && OUTPUTS.keeps("outputs/T-1/sheet.XLSX"), "documents go to the store");
    assert_eq!(hand_in("outputs/T-3 probe/evidence/a.md"), Some(("T-3 probe", "evidence/a.md")));
    assert_eq!(hand_in("outputs/loose.md"), None);
}

#[test]
fn a_hand_in_keeps_its_summary_first_and_no_more_than_its_limit() {
    let mut files: Vec<(String, u64)> = (0..30).map(|n| (format!("evidence/{n:02}.md"), 100)).collect();
    files.push(("REPORT.md".into(), 5_000));
    files.push(("table.csv".into(), 50));
    files.push(("notes.txt".into(), 10));
    files.push(("dump.txt".into(), FILE_KEPT + 1));
    let (kept, over) = pick(&files);
    assert_eq!(kept.len(), FILES_KEPT);
    assert!(kept.contains("REPORT.md") && kept.contains("table.csv") && kept.contains("notes.txt"), "{kept:?}");
    assert!(over.contains("dump.txt") && over.contains("evidence/29.md"), "{over:?}");
    assert_eq!(kept.len() + over.len(), files.len());
    // The same files, the same answer.
    files.reverse();
    assert_eq!(pick(&files).0, kept);
    // A megabyte at most.
    let big: Vec<(String, u64)> = (0..10).map(|n| (format!("{n}.md"), 200_000)).collect();
    let (kept, over) = pick(&big);
    assert_eq!((kept.len(), over.len()), (5, 5));
}

#[test]
fn the_ignore_files_keep_out_exactly_what_they_say() {
    let dir = scratch("ignore");
    git(&dir, &["init", "-q"]).unwrap();
    let root = with_block("mine.txt\n");
    assert_eq!(with_block(&root), root, "once");
    assert!(root.starts_with("mine.txt\n\n# >>> equipe"), "{root}");
    write(&dir.join("outputs/.gitignore"), root.as_bytes());
    for yes in ["outputs/T-1/probe.json", "outputs/T-1/DATA.Json", "outputs/T-1/raw/summary.md", "outputs/T-1/x/node_modules/a.md", "outputs/T-1/dom/page.html", "outputs/mine.txt"] {
        assert!(ignored(&dir, yes), "{yes}");
    }
    for no in ["outputs/T-1/README.md", "outputs/T-1/evidence/notes.txt", "outputs/T-1/raw-notes.md"] {
        assert!(!ignored(&dir, no), "{no}");
    }
    let over: BTreeSet<String> = ["a b.md", "[draft]*?.md", "trailing .md ", "evidence/결과.txt", "#hash.md", "!bang.md"].map(str::to_owned).into();
    let cap = with_cap("# the member's own\n", &over);
    assert_eq!(with_cap(&cap, &over), cap, "once");
    write(&dir.join("outputs/T-1/.gitignore"), cap.as_bytes());
    for path in &over {
        assert!(ignored(&dir, &format!("outputs/T-1/{path}")), "{path}: {cap}");
    }
    for no in ["outputs/T-1/ab.md", "outputs/T-1/d.md", "outputs/T-1/trailing .md", "outputs/T-2/a b.md"] {
        assert!(!ignored(&dir, no), "{no}");
    }
    assert_eq!(listed(&cap), over, "read back as written");
    // Nothing past the limit any more: the block goes, the member's own lines stay.
    assert_eq!(with_cap(&cap, &BTreeSet::new()), "# the member's own\n");
    assert_eq!(with_cap("", &BTreeSet::new()), "");
}

#[test]
fn a_squad_folder_stops_tracking_machine_output_in_one_commit_keeping_every_file() {
    let home = squad_folder();
    let before = tracked(&home, "main").len();
    let migrated = migrate(&home).unwrap().expect("machine output was tracked");
    let files = tracked(&home, "main");
    assert_eq!(migrated.untracked, 4 + 6, "the probe's four files, and six notes past the limit");
    assert_eq!(files.len(), before - migrated.untracked + 2, "and two ignore files: {files:?}");
    assert!(files.iter().all(|f| !is_machine(f)), "{files:?}");
    let t1: Vec<&String> = files.iter().filter(|f| f.starts_with("outputs/T-1 probe/") && !f.ends_with(".gitignore")).collect();
    assert_eq!(t1.len(), FILES_KEPT, "{t1:?}");
    for rel in ["outputs/T-1 probe/README.md", "outputs/T-1 probe/notes/00.md", "outputs/T-2 brief/brief.md", "notes/research.md", "outputs/.gitignore", "outputs/T-1 probe/.gitignore"] {
        assert!(files.contains(&rel.to_owned()), "{rel} in {files:?}");
    }
    for rel in ["outputs/T-1 probe/probe.mjs", "outputs/T-1 probe/data/390-dark.json", "outputs/T-1 probe/dom/page.html", "outputs/T-1 probe/notes/24.md"] {
        assert!(home.join(rel).is_file(), "{rel} stays in the folder");
    }
    assert_eq!(git(&home, &["status", "--porcelain", "--untracked-files=all"]).unwrap(), "", "nothing left for a save to take");
    assert_eq!(migrate(&home).unwrap(), None, "once");
    // A project's own `outputs/` is the project's.
    let repo = scratch("project");
    git(&repo, &["init", "-q", "-b", "main"]).unwrap();
    git(&repo, &["config", "user.name", "Test"]).unwrap();
    git(&repo, &["config", "user.email", "test@example.com"]).unwrap();
    write(&repo.join("outputs/data.json"), b"{}");
    commit_all(&repo, "data", &[]);
    assert_eq!(migrate(&repo).unwrap(), None);
    assert_eq!(tracked(&repo, "main"), ["outputs/data.json"]);
}

#[test]
fn a_tasks_hand_in_lands_with_its_summary_in_git_and_its_bulk_on_disk() {
    let home = squad_folder();
    migrate(&home).unwrap();
    let equipe = scratch("worktrees");
    let s = worktree::create_after(&home, &equipe, "t3-abc-1", "qa", &[], &worktree::home_patterns(&[3])).unwrap();
    let hand_in = s.path.join("outputs/T-3 widths");
    write(&hand_in.join("README.md"), b"# Widths\n\nAll pass; the matrix is in raw/.");
    write(&hand_in.join("probe.json"), b"{\"390\": \"ok\"}");
    write(&hand_in.join("raw/1280-dark.md"), b"ok");
    write(&hand_in.join("probe.mjs"), b"run()");
    for n in 0..22 {
        write(&hand_in.join(format!("checks/{n:02}.txt")), b"pass");
    }
    s.commit_all("widths").unwrap().unwrap();
    let files = tracked(&home, &s.branch);
    let mine: Vec<&String> = files.iter().filter(|f| f.starts_with("outputs/T-3 widths/") && !f.ends_with(".gitignore")).collect();
    assert_eq!(mine.len(), FILES_KEPT, "{mine:?}");
    assert!(mine.contains(&&"outputs/T-3 widths/README.md".to_owned()));
    assert!(!files.iter().any(|f| is_machine(f)), "{files:?}");
    // A member forces a dump in anyway: taken out of its commit before it lands.
    write(&hand_in.join("dom.html"), b"<html>");
    commit_all(&s.path, "dom", &["outputs/T-3 widths/dom.html"]);
    let blob = git(&s.path, &["rev-parse", "HEAD:outputs/T-3 widths/dom.html"]).unwrap();
    worktree::merge_into_main(&home, &s.branch, "merge").unwrap();
    assert!(!git(&home, &["rev-list", "--objects", "main"]).unwrap().contains(blob.as_str()), "it never reached the folder's history");
    let report = super::super::take_report(&home, &s.branch).expect("its member hears");
    assert_eq!(report.raw_kept_out, 1);
    let line = report.line(3);
    assert!(line.contains("raw/") && line.contains("README.md") && line.contains("never `git add -f`"), "{line}");
    // Its bulk is in the folder all the same, and nothing there is left for a save to take.
    for rel in ["probe.json", "raw/1280-dark.md", "probe.mjs", "dom.html", "checks/21.txt"] {
        assert!(home.join("outputs/T-3 widths").join(rel).is_file(), "{rel} carried into the folder");
    }
    assert_eq!(git(&home, &["status", "--porcelain", "--untracked-files=all"]).unwrap(), "");
    assert!(home.join("outputs/T-1 probe/probe.mjs").is_file(), "earlier hand-ins untouched");
}

#[test]
fn landing_never_takes_out_what_main_tracks() {
    let home = squad_folder();
    // Not upgraded yet (the guard ran on the daemon's loop): main tracks the probe's dumps.
    let equipe = scratch("worktrees");
    let s = worktree::create_after(&home, &equipe, "t1-abc-2", "qa", &[], &worktree::home_patterns(&[1])).unwrap();
    write(&s.path.join("outputs/T-1 probe/data/390-dark.json"), b"{\"changed\": true}");
    write(&s.path.join("outputs/T-1 probe/data/new.json"), b"{}");
    commit_all(&s.path, "probe again", &["outputs/T-1 probe/data/new.json"]);
    let main = worktree::main_branch(&home).unwrap();
    let (count, _) = guard_in(&home, &s.branch, &main).unwrap();
    assert_eq!(count, 1, "only the file the branch added");
    let files = tracked(&home, &s.branch);
    assert!(files.contains(&"outputs/T-1 probe/data/390-dark.json".to_owned()) && !files.contains(&"outputs/T-1 probe/data/new.json".to_owned()), "{files:?}");
}

#[test]
fn machine_output_goes_after_its_days_and_nothing_else_does() {
    let home = squad_folder();
    migrate(&home).unwrap();
    let store = Store::of(&home).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(RAW_KEPT + 86_400);
    let age = |rel: &str| std::fs::File::options().write(true).open(home.join(rel)).unwrap().set_modified(old).unwrap();
    for rel in ["outputs/T-1 probe/probe.mjs", "outputs/T-1 probe/data/390-dark.json", "outputs/T-1 probe/data/run.log", "outputs/T-1 probe/notes/24.md", "outputs/T-1 probe/README.md"] {
        age(rel);
    }
    assert!(sweep(&home).is_empty(), "kept a while from when equipe started keeping it out of git");
    std::fs::write(store.root().join(SINCE), (now() - RAW_KEPT - 86_400).to_string()).unwrap();
    let swept = sweep(&home);
    assert_eq!(swept.len(), 1, "{swept:?}");
    assert!(swept[0].starts_with("3 files"), "{swept:?}");
    for gone in ["outputs/T-1 probe/probe.mjs", "outputs/T-1 probe/data/390-dark.json", "outputs/T-1 probe/data/run.log"] {
        assert!(!home.join(gone).exists(), "{gone}");
    }
    assert!(!home.join("outputs/T-1 probe/data").exists(), "the folder it left empty goes too");
    assert!(home.join("outputs/T-1 probe/dom/page.html").is_file(), "newer machine output stays");
    assert!(home.join("outputs/T-1 probe/notes/24.md").is_file(), "what is past a hand-in's limit is never swept");
    assert!(home.join("outputs/T-1 probe/README.md").is_file(), "nor what git tracks");
    assert_eq!(git(&home, &["status", "--porcelain"]).unwrap(), "");
}
