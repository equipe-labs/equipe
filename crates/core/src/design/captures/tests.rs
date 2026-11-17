use super::*;
use crate::worktree::{self, git};

fn scratch(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-captures-{name}-{}-{n}", std::process::id()));
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

/// A project whose design folder equipe keeps, its UI map's pictures
/// committed the way older equipe did, and a logo of its own.
fn project() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]).unwrap();
    git(&dir, &["config", "user.name", "Test"]).unwrap();
    git(&dir, &["config", "user.email", "test@example.com"]).unwrap();
    write(&dir.join("README.md"), b"hello\n");
    write(&dir.join("design/tokens.json"), b"{}");
    write(&dir.join("design/flows/flows.json"), br#"{ "flows": [] }"#);
    write(&dir.join("design/flows/login/form/default.light.390.png"), b"light 390");
    write(&dir.join("design/flows/login/form/default.dark.390.png"), b"dark 390");
    write(&dir.join("design/qa/t1/after.png"), b"qa after");
    write(&dir.join("design/qa/t1/report.md"), b"# QA");
    write(&dir.join("design/assets/logo/mark.png"), b"the logo");
    commit_all(&dir, "init", &[]);
    dir
}

fn tracked(dir: &Path, rev: &str) -> Vec<String> {
    git(dir, &["ls-tree", "-r", "--name-only", rev]).unwrap().lines().map(str::to_owned).collect()
}

#[test]
fn a_capture_is_a_picture_or_video_in_one_of_the_design_folders_tools_write() {
    for yes in ["design/flows/login/form/error.dark.390.png", "design/qa/t1/after.PNG", "design/directions/t7/boards/a-1440.webp", "design/flows/onboarding/walkthrough.light.390.mp4", "design/references/R1/1280.jpg"] {
        assert!(is_capture(yes), "{yes}");
    }
    for no in ["design/flows/flows.json", "design/assets/logo/mark.png", "design/components/button.png", "src/design/flows/a/b.png", "design/flows.png", "design/qa/t1/report.md", "design/flows/../x.png"] {
        assert!(!is_capture(no), "{no}");
    }
    assert_eq!(ticket_of_branch("equipe/designer-t42-19a2b3c-3"), Some(42));
    assert_eq!(ticket_of_branch("equipe/try-19a2b3c-4f"), None);
}

#[test]
fn the_ignore_block_is_added_once_kept_current_and_leaves_the_rest() {
    let block = ignore_block();
    assert!(block.contains("/flows/**/*.[pP][nN][gG]\n") && block.contains("/directions/**/*.[mM][pP][44]\n"));
    assert!(!block.contains("/assets/"));
    let added = with_block("*.log\n");
    assert!(added.starts_with("*.log\n\n# >>> equipe"));
    assert_eq!(with_block(&added), added, "once");
    let stale = added.replace("/flows/**/*.[pP][nN][gG]\n", "/old/**\n") + "mine\n";
    let fixed = with_block(&stale);
    assert_eq!(fixed, format!("{added}mine\n"), "replaced where it was, the rest kept");
    assert_eq!(with_block(""), block);
}

#[test]
fn a_blob_id_is_gits() {
    let dir = project();
    let file = dir.join("design/flows/login/form/default.light.390.png");
    let ours = blob_id(&file, Kind::Sha1).unwrap();
    assert_eq!(ours.0, git(&dir, &["hash-object", "--", &file.to_string_lossy()]).unwrap());
    assert_eq!(ours.1, 9);
}

#[test]
fn the_manifest_is_one_line_a_capture_and_says_what_a_map_picture_is() {
    let mut m = Manifest::default();
    let e = Entry::new("flows/login/form/error.dark.390.png", &"a".repeat(40), 12, 1_700_000_000, Some(7));
    assert_eq!((e.flow.as_deref(), e.screen.as_deref(), e.state.as_deref(), e.mode.as_deref(), e.width), (Some("login"), Some("form"), Some("error"), Some("dark"), Some(390)));
    m.entries.insert(e.path.clone(), e);
    let other = Entry::new("qa/t1/after.png", &"b".repeat(40), 3, 1_700_000_001, None);
    assert_eq!(other.flow, None);
    m.entries.insert(other.path.clone(), other);
    let text = m.render();
    assert_eq!(text.lines().count(), 4);
    assert!(!text.contains("/Users/") && !text.contains("\"ticket\":null"));
    assert_eq!(Manifest::parse(&text), Some(m));
    assert_eq!(Manifest::parse("<<<<<<< ours\n"), None);
    let hostile = r#"{"captures": [{"path": "../../etc/x.png", "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1, "created_at": 1}]}"#;
    assert!(Manifest::parse(hostile).unwrap().is_empty(), "a path out of the design folder is no capture");
}

#[test]
fn manifests_merge_entry_by_entry_the_newest_render_winning() {
    let e = |path: &str, hash: char, at: u64| Entry::new(path, &hash.to_string().repeat(40), 1, at, None);
    let m = |entries: &[Entry]| Manifest { entries: entries.iter().map(|e| (e.path.clone(), e.clone())).collect() };
    let base = m(&[e("qa/a.png", 'a', 1), e("qa/b.png", 'b', 1), e("qa/c.png", 'c', 1)]);
    // Ours re-renders a and removes c; theirs re-renders b and a too, later, and adds d.
    let ours = m(&[e("qa/a.png", 'd', 5), e("qa/b.png", 'b', 1)]);
    let theirs = m(&[e("qa/a.png", 'e', 9), e("qa/b.png", 'f', 3), e("qa/c.png", 'c', 1), e("qa/d.png", '1', 4)]);
    let merged = merge3(&base, &ours, &theirs);
    let hashes: Vec<(&str, char)> = merged.entries.values().map(|e| (e.path.as_str(), e.hash.chars().next().unwrap())).collect();
    assert_eq!(hashes, [("qa/a.png", 'e'), ("qa/b.png", 'f'), ("qa/d.png", '1')]);
}

#[test]
fn an_upgrade_untracks_main_s_captures_in_one_commit_keeping_every_file() {
    let repo = project();
    let migrated = migrate(&repo).unwrap().expect("captures were tracked");
    assert_eq!(migrated.untracked, 3);
    let files = tracked(&repo, "main");
    assert!(files.iter().all(|f| !is_capture(f)), "{files:?}");
    assert!(files.contains(&"design/assets/logo/mark.png".to_owned()) && files.contains(&"design/qa/t1/report.md".to_owned()));
    assert!(files.contains(&IGNORE_PATH.to_owned()) && files.contains(&MANIFEST_PATH.to_owned()));
    // The checkout followed, every picture still there, nothing to commit.
    for rel in ["design/flows/login/form/default.light.390.png", "design/flows/login/form/default.dark.390.png", "design/qa/t1/after.png"] {
        assert!(repo.join(rel).is_file(), "{rel}");
    }
    assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "");
    let manifest = manifest_at(&repo, "main");
    assert_eq!(manifest.entries.len(), 3);
    let store = Store::of(&repo).unwrap();
    assert!(manifest.entries.values().all(|e| store.has(&e.hash)), "each kept in the store");
    assert!(store.root().starts_with(repo.join(".git")));
    // History as it was: the first commit still holds them.
    assert_eq!(git(&repo, &["rev-list", "--count", "main"]).unwrap(), "2");
    assert!(tracked(&repo, "main~1").iter().any(|f| is_capture(f)));
    assert_eq!(migrate(&repo).unwrap(), None, "once");
}

#[test]
fn a_project_without_an_equipe_design_folder_is_left_alone() {
    let dir = scratch("plain");
    git(&dir, &["init", "-q", "-b", "main"]).unwrap();
    git(&dir, &["config", "user.name", "Test"]).unwrap();
    git(&dir, &["config", "user.email", "test@example.com"]).unwrap();
    write(&dir.join("design/mockups/hero.png"), b"theirs");
    commit_all(&dir, "init", &[]);
    assert_eq!(migrate(&dir).unwrap(), None);
    assert!(tracked(&dir, "main").contains(&"design/mockups/hero.png".to_owned()));
}

#[test]
fn a_ticket_s_pictures_go_from_its_worktree_to_the_main_checkout_through_the_store_never_through_git() {
    let repo = project();
    migrate(&repo).unwrap();
    let home = scratch("home");
    let s = worktree::create(&repo, &home, "t5-abc-1", "designer").unwrap();
    // Its worktree has the map's pictures, though git never checked them out.
    let light = "design/flows/login/form/default.light.390.png";
    assert_eq!(std::fs::read(s.path.join(light)).unwrap(), b"light 390");
    // Re-shot one, shot a new state, removed another.
    write(&s.path.join(light), b"light 390, again");
    write(&s.path.join("design/flows/login/form/error.light.390.png"), b"error 390");
    std::fs::remove_file(s.path.join("design/qa/t1/after.png")).unwrap();
    let commit = s.commit_all("map refreshed").unwrap().expect("its manifest changed");
    assert!(tracked(&s.path, &commit).iter().all(|f| !is_capture(f)));
    let manifest = manifest_at(&s.path, &commit);
    assert_eq!(manifest.entries.keys().collect::<Vec<_>>(), ["flows/login/form/default.dark.390.png", "flows/login/form/default.light.390.png", "flows/login/form/error.light.390.png"]);
    assert_eq!(manifest.entries["flows/login/form/error.light.390.png"].ticket, Some(5));
    // What its hand-in shows.
    let mut shown = changed(&s.path, &s.base);
    shown.sort();
    assert_eq!(shown, [light.to_owned(), "design/flows/login/form/error.light.390.png".to_owned()]);
    worktree::merge_into_main(&repo, &s.branch, "merge").unwrap();
    assert_eq!(std::fs::read(repo.join(light)).unwrap(), b"light 390, again");
    assert_eq!(std::fs::read(repo.join("design/flows/login/form/error.light.390.png")).unwrap(), b"error 390");
    assert!(!repo.join("design/qa/t1/after.png").exists(), "what it removed went");
    assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "");
    assert_eq!(take_report(&repo, &s.branch), None, "nothing to tell its member");
}

#[test]
fn captures_committed_anyway_are_taken_out_of_the_branch_s_own_commits_before_it_lands() {
    let repo = project();
    migrate(&repo).unwrap();
    let home = scratch("home");
    let s = worktree::create(&repo, &home, "t9-abc-1", "designer").unwrap();
    // A member forces pictures in, over two commits, and writes code.
    write(&s.path.join("design/mockups/t9/a-1280.png"), b"mockup one");
    commit_all(&s.path, "mockup", &["design/mockups/t9/a-1280.png"]);
    write(&s.path.join("design/mockups/t9/a-1280.png"), b"mockup two");
    write(&s.path.join("src/app.ts"), b"code");
    commit_all(&s.path, "mockup again, and code", &["design/mockups/t9/a-1280.png"]);
    let blobs = [git(&s.path, &["rev-parse", "HEAD~1:design/mockups/t9/a-1280.png"]).unwrap(), git(&s.path, &["rev-parse", "HEAD:design/mockups/t9/a-1280.png"]).unwrap()];
    worktree::merge_into_main(&repo, &s.branch, "merge").unwrap();
    let objects = git(&repo, &["rev-list", "--objects", "main"]).unwrap();
    for blob in &blobs {
        assert!(!objects.contains(blob.as_str()), "{blob} reached main's history");
    }
    assert!(tracked(&repo, "main").contains(&"src/app.ts".to_owned()), "the code landed");
    assert_eq!(git(&repo, &["log", "--format=%s", "-2", &s.branch]).unwrap(), "mockup again, and code\nmockup", "the branch's commits, rewritten");
    assert_eq!(std::fs::read(repo.join("design/mockups/t9/a-1280.png")).unwrap(), b"mockup two", "in the main checkout");
    assert!(manifest_at(&repo, "main").entries.contains_key("mockups/t9/a-1280.png"));
    // Its worktree follows the branch and keeps the file, untracked and ignored.
    assert_eq!(git(&s.path, &["status", "--porcelain"]).unwrap(), "");
    assert!(s.path.join("design/mockups/t9/a-1280.png").is_file());
    let report = take_report(&repo, &s.branch).expect("its member hears");
    assert_eq!((report.kept_out, report.bytes), (1, 10));
    assert!(report.line(9).contains("never `git add` them"));
    assert_eq!(take_report(&repo, &s.branch), None, "once");
}

#[test]
fn a_commit_heavy_with_other_images_is_reported() {
    let repo = project();
    migrate(&repo).unwrap();
    git(&repo, &["checkout", "-q", "-b", "work"]).unwrap();
    write(&repo.join("docs/screens/big.png"), &vec![7u8; (HEAVY_COMMIT + 1) as usize]);
    commit_all(&repo, "screens for the docs", &[]);
    git(&repo, &["checkout", "-q", "main"]).unwrap();
    let report = guard(&repo, "work").unwrap();
    assert_eq!(report.kept_out, 0);
    assert_eq!(report.heavy.len(), 1);
    assert_eq!(report.heavy[0].subject, "screens for the docs");
    assert!(report.line(3).contains("10.5 MB"), "{}", report.line(3));
}

#[test]
fn two_branches_re_rendering_the_map_both_land_their_manifests_merged() {
    let repo = project();
    migrate(&repo).unwrap();
    let home = scratch("home");
    let (a, b) = (worktree::create(&repo, &home, "t1-a-1", "designer").unwrap(), worktree::create(&repo, &home, "t2-b-1", "designer").unwrap());
    write(&a.path.join("design/flows/login/form/default.light.390.png"), b"from a");
    write(&b.path.join("design/flows/login/form/default.dark.390.png"), b"from b");
    write(&b.path.join("design/flows/login/form/default.light.390.png"), b"from b, later");
    a.commit_all("a").unwrap().unwrap();
    b.commit_all("b").unwrap().unwrap();
    worktree::merge_into_main(&repo, &a.branch, "merge a").unwrap();
    assert_eq!(worktree::merge_conflicts(&repo, &b.branch).unwrap(), Vec::<String>::new(), "the manifest settles itself");
    worktree::merge_into_main(&repo, &b.branch, "merge b").unwrap();
    let manifest = manifest_at(&repo, "main");
    let hash = |rel: &str| manifest.entries[rel].hash.clone();
    let id = |bytes: &[u8]| {
        let file = repo.join(".git/scratch-blob");
        std::fs::write(&file, bytes).unwrap();
        blob_id(&file, Kind::Sha1).unwrap().0
    };
    assert_eq!(hash("flows/login/form/default.dark.390.png"), id(b"from b"));
    assert_eq!(std::fs::read(repo.join("design/flows/login/form/default.dark.390.png")).unwrap(), b"from b");
    assert!(git(&repo, &["show", &format!("main:{MANIFEST_PATH}")]).unwrap().lines().all(|l| !l.starts_with("<<<<<<<")));
}

#[test]
fn a_new_worktree_of_a_main_that_moved_on_gets_its_pictures_from_the_main_checkout() {
    let repo = project();
    migrate(&repo).unwrap();
    // The store lost its objects (another machine, a cleaned folder): the main checkout still has the bytes.
    let store = Store::of(&repo).unwrap();
    std::fs::remove_dir_all(store.root().join("objects")).unwrap();
    let home = scratch("home");
    let s = worktree::create(&repo, &home, "t3-x-1", "designer").unwrap();
    assert_eq!(std::fs::read(s.path.join("design/qa/t1/after.png")).unwrap(), b"qa after");
    assert!(store.has(&manifest_at(&repo, "main").entries["qa/t1/after.png"].hash));
}

#[test]
fn the_store_forgets_what_no_manifest_names_once_old() {
    let repo = project();
    migrate(&repo).unwrap();
    let store = Store::of(&repo).unwrap();
    let stray = repo.join(".git/stray");
    std::fs::write(&stray, b"nobody's").unwrap();
    let (hash, _) = blob_id(&stray, Kind::Sha1).unwrap();
    store.put_file(&stray, &hash).unwrap();
    assert!(gc(&repo).is_empty(), "new: kept a while");
    let old = SystemTime::now() - std::time::Duration::from_secs(UNNAMED_KEPT + 60);
    std::fs::File::options().write(true).open(store.object(&hash)).unwrap().set_modified(old).unwrap();
    let named = manifest_at(&repo, "main").entries.values().next().unwrap().hash.clone();
    std::fs::File::options().write(true).open(store.object(&named)).unwrap().set_modified(old).unwrap();
    assert_eq!(gc(&repo).len(), 1);
    assert!(!store.has(&hash) && store.has(&named), "what main names stays");
}
