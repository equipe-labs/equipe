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
    // What used to be read from git at a commit is read from the store now.
    let kept = file_at(&repo, "main", "design/qa/t1/after.png").unwrap();
    assert_eq!(std::fs::read(kept).unwrap(), b"qa after");
    assert_eq!(file_at(&repo, "main", "design/qa/t1/report.md"), None);
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
    // Its worktree holds none of main's pictures: they are read from the main checkout.
    let light = "design/flows/login/form/default.light.390.png";
    assert!(on_disk(&s.path).is_empty(), "a new worktree costs nothing for the map");
    assert_eq!(lent(&s.path, light), Some(repo.join(light)));
    assert_eq!(borrowed(&s.path.join("design")).len(), 3);
    // Re-shot one and shot a new state.
    write(&s.path.join(light), b"light 390, again");
    write(&s.path.join("design/flows/login/form/error.light.390.png"), b"error 390");
    assert_eq!(lent(&s.path, light), Some(s.path.join(light)), "its own, once it has one");
    let commit = s.commit_all("map refreshed").unwrap().expect("its manifest changed");
    assert!(tracked(&s.path, &commit).iter().all(|f| !is_capture(f)));
    let manifest = manifest_at(&s.path, &commit);
    assert_eq!(manifest.entries.keys().collect::<Vec<_>>(), ["flows/login/form/default.dark.390.png", "flows/login/form/default.light.390.png", "flows/login/form/error.light.390.png", "qa/t1/after.png"], "what it never had stays listed");
    assert_eq!(manifest.entries["flows/login/form/error.light.390.png"].ticket, Some(5));
    // What its hand-in shows.
    let mut shown = changed(&s.path, &s.base);
    shown.sort();
    assert_eq!(shown, [light.to_owned(), "design/flows/login/form/error.light.390.png".to_owned()]);
    worktree::merge_into_main(&repo, &s.branch, "merge").unwrap();
    assert_eq!(std::fs::read(repo.join(light)).unwrap(), b"light 390, again");
    assert_eq!(std::fs::read(repo.join("design/flows/login/form/error.light.390.png")).unwrap(), b"error 390");
    assert_eq!(std::fs::read(repo.join("design/qa/t1/after.png")).unwrap(), b"qa after");
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
fn a_reopened_ticket_s_worktree_gets_back_its_own_pictures_and_only_those() {
    let repo = project();
    migrate(&repo).unwrap();
    let home = scratch("home");
    let s = worktree::create(&repo, &home, "t3-x-1", "designer").unwrap();
    write(&s.path.join("design/flows/login/form/error.dark.390.png"), b"its own");
    s.commit_all("error state").unwrap().unwrap();
    s.remove_worktree().unwrap();
    let again = worktree::resume(&repo, &home, "t3-x-2", &s.branch, &s.base, &[]).unwrap();
    assert_eq!(std::fs::read(again.path.join("design/flows/login/form/error.dark.390.png")).unwrap(), b"its own");
    assert_eq!(on_disk(&again.path).len(), 1, "main's, unchanged, stay in the main checkout");
    assert_eq!(lent(&again.path, "design/qa/t1/after.png"), Some(repo.join("design/qa/t1/after.png")));
}

#[test]
fn a_worktree_s_ui_map_shows_main_s_unchanged_pictures_and_its_own() {
    let repo = project();
    write(&repo.join("design/flows/flows.json"), br#"{ "flows": [ { "id": "login", "title": "Log in", "screens": [ { "id": "form", "title": "Form", "states": [ { "id": "default" }, { "id": "error" } ] } ] } ] }"#);
    commit_all(&repo, "the map", &[]);
    migrate(&repo).unwrap();
    let home = scratch("home");
    let s = worktree::create(&repo, &home, "t4-x-1", "designer").unwrap();
    write(&s.path.join("design/flows/login/form/error.light.390.png"), b"new state");
    let map = crate::design::flows::read(&s.path.join("design"), Some(&s.path), crate::i18n::Lang::En).unwrap();
    let states = &map.flows[0].screens[0].states;
    let paths = |i: usize| states[i].pictures.iter().map(|p| p.path.clone()).collect::<Vec<_>>();
    let main_design = repo.join("design").canonicalize().unwrap();
    assert_eq!(paths(0).len(), 2);
    assert!(paths(0).iter().all(|p| p.starts_with(main_design.to_str().unwrap())), "{:?}", paths(0));
    assert_eq!(paths(1), [s.path.join("design/flows/login/form/error.light.390.png").to_string_lossy().into_owned()]);
    // A state renamed away takes its pictures out of the manifest.
    write(&s.path.join("design/flows/flows.json"), br#"{ "flows": [ { "id": "login", "title": "Log in", "screens": [ { "id": "form", "title": "Form", "states": [ { "id": "error" } ] } ] } ] }"#);
    let commit = s.commit_all("default goes").unwrap().unwrap();
    assert_eq!(manifest_at(&s.path, &commit).entries.keys().filter(|k| k.starts_with("flows/")).collect::<Vec<_>>(), ["flows/login/form/error.light.390.png"]);
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

#[test]
fn an_object_is_dated_when_it_came_into_the_store_not_by_its_file() {
    let repo = project();
    let store = Store::of(&repo).unwrap();
    let old_file = repo.join(".git/old-file");
    std::fs::write(&old_file, b"made long ago").unwrap();
    let old = SystemTime::now() - std::time::Duration::from_secs(UNNAMED_KEPT + 86_400);
    std::fs::File::options().write(true).open(&old_file).unwrap().set_modified(old).unwrap();
    let (hash, _) = blob_id(&old_file, Kind::Sha1).unwrap();
    store.put_file(&old_file, &hash).unwrap();
    assert!(gc(&repo).is_empty(), "kept a while from when it was stored");
    assert!(store.has(&hash));
}

// ── a squad's folder: what its tasks hand over ──

/// A squad's own folder (`…/squads/acme/home`) the way an older equipe kept
/// it: a task's hand-in pictures and video committed beside its notes.
fn squad_folder() -> PathBuf {
    let dir = scratch("equipe").join("squads/acme/home");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]).unwrap();
    git(&dir, &["config", "user.name", "equipe"]).unwrap();
    git(&dir, &["config", "user.email", "equipe@localhost"]).unwrap();
    write(&dir.join("notes/research.md"), b"# Research");
    write(&dir.join("outputs/T-1 brief/board.png"), b"board one");
    write(&dir.join("outputs/T-1 brief/walkthrough.MP4"), b"a video");
    write(&dir.join("outputs/T-1 brief/notes.md"), b"# Brief");
    commit_all(&dir, "the squad's folder", &[]);
    dir
}

#[test]
fn a_squad_folders_hand_ins_are_its_outputs_pictures_videos_and_generated_files() {
    for yes in ["outputs/T-1 brief/board.png", "outputs/T-12/shots/a.JPG", "outputs/report.pdf", "outputs/T-3/clip.mov", "outputs/T-3/deck.zip"] {
        assert!(OUTPUTS.keeps(yes), "{yes}");
    }
    for no in ["outputs/T-1 brief/notes.md", "outputs/.captures.json", "outputs/.gitignore", "notes/a.png", "outputs.png", "outputs/../x.png", "src/outputs/a.png"] {
        assert!(!OUTPUTS.keeps(no), "{no}");
    }
    let block = OUTPUTS.ignore_block();
    assert!(block.contains("\n*.[pP][nN][gG]\n") && block.contains("\n*.[pP][dD][fF]\n") && !block.contains("/flows/"), "{block}");
    assert_eq!(OUTPUTS.with_block(&OUTPUTS.with_block("mine\n")), OUTPUTS.with_block("mine\n"), "once");
    // The design folder's captures are what they were.
    assert!(is_capture("design/qa/t1/after.png") && !is_capture("outputs/T-1/a.png"));
    assert_eq!(ignore_block(), DESIGN.ignore_block());
}

#[test]
fn a_squad_folder_stops_tracking_its_hand_in_pictures_in_one_commit_keeping_every_file() {
    let home = squad_folder();
    let migrated = migrate_in(&OUTPUTS, &home).unwrap().expect("hand-ins were tracked");
    assert_eq!(migrated.untracked, 2);
    let files = tracked(&home, "main");
    assert!(files.iter().all(|f| !OUTPUTS.keeps(f)), "{files:?}");
    for rel in ["outputs/T-1 brief/notes.md", "notes/research.md", OUTPUTS.ignore_path, OUTPUTS.manifest_path] {
        assert!(files.contains(&rel.to_owned()), "{rel} in {files:?}");
    }
    for rel in ["outputs/T-1 brief/board.png", "outputs/T-1 brief/walkthrough.MP4"] {
        assert!(home.join(rel).is_file(), "{rel} stays in the folder");
    }
    assert_eq!(git(&home, &["status", "--porcelain"]).unwrap(), "");
    let manifest = manifest_at_in(&OUTPUTS, &home, "main");
    assert_eq!(manifest.entries.keys().collect::<Vec<_>>(), ["T-1 brief/board.png", "T-1 brief/walkthrough.MP4"]);
    let store = Store::of(&home).unwrap();
    assert!(manifest.entries.values().all(|e| store.has(&e.hash)), "each kept in the store");
    assert_eq!(migrate_in(&OUTPUTS, &home).unwrap(), None, "once");
    // The store keeps them: no manifest of the design folder names them, and gc leaves them.
    assert!(gc(&home).is_empty());
    assert!(manifest.entries.values().all(|e| store.has(&e.hash)));
    // A project's own `outputs/` is the project's: left as it is.
    let repo = project();
    write(&repo.join("outputs/chart.png"), b"chart");
    commit_all(&repo, "chart", &[]);
    assert_eq!(migrate_in(&OUTPUTS, &repo).unwrap(), None);
    assert!(tracked(&repo, "main").contains(&"outputs/chart.png".to_owned()));
}

#[test]
fn a_tasks_hand_in_lands_in_the_squads_folder_without_its_pictures_in_history() {
    let home = squad_folder();
    migrate_in(&OUTPUTS, &home).unwrap();
    let equipe = scratch("worktrees");
    let s = worktree::create_after(&home, &equipe, "t2-abc-1", "designer", &[], &worktree::home_patterns(&[2])).unwrap();
    assert!(!s.path.join("outputs/T-1 brief").exists(), "sparse: other tickets' folders are left out");
    write(&s.path.join("outputs/T-2 gallery/hero.png"), b"hero");
    write(&s.path.join("outputs/T-2 gallery/README.md"), b"# Gallery");
    s.commit_all("gallery").unwrap().unwrap();
    let files = tracked(&home, &s.branch);
    assert!(files.contains(&"outputs/T-2 gallery/README.md".to_owned()) && !files.contains(&"outputs/T-2 gallery/hero.png".to_owned()), "{files:?}");
    assert!(files.contains(&"outputs/T-1 brief/notes.md".to_owned()), "what it left out stays as main has it");
    assert_eq!(changed(&s.path, &s.base), ["outputs/T-2 gallery/hero.png"], "what its hand-in shows");
    // A member forces one in anyway: taken out of its commit before it lands.
    write(&s.path.join("outputs/T-2 gallery/extra.webp"), b"extra");
    commit_all(&s.path, "extra", &["outputs/T-2 gallery/extra.webp"]);
    let blob = git(&s.path, &["rev-parse", "HEAD:outputs/T-2 gallery/extra.webp"]).unwrap();
    worktree::merge_into_main(&home, &s.branch, "merge").unwrap();
    assert!(!git(&home, &["rev-list", "--objects", "main"]).unwrap().contains(blob.as_str()), "it never reached the folder's history");
    assert_eq!(std::fs::read(home.join("outputs/T-2 gallery/hero.png")).unwrap(), b"hero", "in the folder all the same");
    assert_eq!(std::fs::read(home.join("outputs/T-2 gallery/extra.webp")).unwrap(), b"extra");
    assert!(home.join("outputs/T-1 brief/board.png").is_file(), "earlier hand-ins untouched");
    let report = take_report(&home, &s.branch).expect("its member hears");
    assert_eq!((report.handed_in_kept_out, report.kept_out), (1, 0));
    assert!(report.line(2).contains("outputs/") && report.line(2).contains("never `git add` them"), "{}", report.line(2));
    let kept = file_at(&home, "main", "outputs/T-2 gallery/hero.png").expect("read back from the store");
    assert_eq!(std::fs::read(kept).unwrap(), b"hero");
}
