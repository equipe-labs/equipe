use super::*;

use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-own-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("design")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("design/taste.md"), "# Taste\n\n## Notes\n- calm\n").unwrap();
    std::fs::write(dir.join("design/tokens.json"), "{}\n").unwrap();
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "base"]);
    dir.canonicalize().unwrap()
}

fn status(dir: &Path) -> String {
    git(dir, &["status", "--porcelain", "--untracked-files=all"])
}

#[test]
fn equipes_own_files_are_known_by_their_place() {
    for own in ["design/taste.md", "web/design/taste.md", "design/decisions/2026-11-17-a.md", "design/decisions/2026-11-17-a.png", "design/critiques/1794749173.md", "design/references/button/notes.md", "design/tuner/signals.jsonl"] {
        assert!(is_own(own), "{own}");
    }
    for not in ["taste.md", "design/tokens.json", "design/flows/flows.json", "design/critiques/a.png", "design/references/board.json", "design/guidelines.md", "src/design/x.ts", "prod-empty.light.1280.png"] {
        assert!(!is_own(not), "{not}");
    }
}

#[test]
fn a_file_equipe_writes_in_the_main_checkout_is_committed_on_main_at_once() {
    let dir = repo("land");
    let taste = dir.join("design/taste.md");
    std::fs::write(&taste, "# Taste\n\n## Style profile\n- Tone: calm\n<!-- /style profile -->\n\n## Notes\n- calm\n").unwrap();
    let decisions = dir.join("design/decisions");
    std::fs::create_dir_all(&decisions).unwrap();
    std::fs::write(decisions.join("2026-11-17-hero.md"), "## Hero\n").unwrap();
    let before = git(&dir, &["rev-parse", "main"]);
    let landed = land(&dir.join("design"), &[taste.clone(), decisions.join("2026-11-17-hero.md")], "equipe: the style profile").unwrap();
    assert_eq!(landed, Landed::Committed(vec!["design/taste.md".into(), "design/decisions/2026-11-17-hero.md".into()]));
    assert_eq!(status(&dir), "", "nothing left uncommitted");
    assert_eq!(git(&dir, &["rev-parse", "main~1"]), before, "one commit on main");
    assert_eq!(git(&dir, &["log", "-1", "--format=%s"]), "equipe: the style profile");
    // Written again unchanged: nothing more.
    assert_eq!(land(&dir.join("design"), &[taste], "again").unwrap(), Landed::Nothing);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_persons_own_work_in_the_checkout_stays_as_it_is() {
    let dir = repo("person");
    std::fs::write(dir.join("README.md"), "the person's edit\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "scratch\n").unwrap();
    let taste = dir.join("design/taste.md");
    std::fs::write(&taste, "# Taste\n\n## Notes\n- calm\n- quiet\n").unwrap();
    land(&dir.join("design"), std::slice::from_ref(&taste), "equipe: taste notes").unwrap();
    assert_eq!(status(&dir), "M README.md\n?? notes.txt", "trimmed: ` M README.md`");
    assert_eq!(git(&dir, &["show", "HEAD:README.md"]), "base", "never committed");
    // Staged by the person differently: theirs to commit, not equipe's.
    std::fs::write(&taste, "staged\n").unwrap();
    git(&dir, &["add", "design/taste.md"]);
    std::fs::write(&taste, "on disk\n").unwrap();
    assert_eq!(land(&dir.join("design"), &[taste], "equipe: taste notes").unwrap(), Landed::Nothing);
    assert!(status(&dir).contains("MM design/taste.md"), "{}", status(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_a_tickets_worktree_nothing_is_committed_for_the_member() {
    let dir = repo("worktree");
    let wt = dir.parent().unwrap().join(format!("equipe-own-worktree-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt);
    git(&dir, &["worktree", "add", "-q", "-b", "work", &wt.to_string_lossy()]);
    let taste = wt.join("design/taste.md");
    std::fs::write(&taste, "changed\n").unwrap();
    assert!(matches!(land(&wt.join("design"), &[taste], "x").unwrap(), Landed::Left(_)));
    assert_eq!(git(&wt, &["status", "--porcelain"]), "M design/taste.md");
    git(&dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_an_older_equipe_left_uncommitted_no_longer_blocks_a_merge() {
    // T-2968: main checked out with design/taste.md changed by equipe, and
    // the accepted branch changing taste.md too — its fast-forward refused.
    let dir = repo("pending");
    git(&dir, &["branch", "work"]);
    git(&dir, &["checkout", "-q", "work"]);
    std::fs::write(dir.join("design/taste.md"), "# Taste\n\n## Notes\n- calm\n- the login screen: centred\n").unwrap();
    git(&dir, &["commit", "-qam", "work"]);
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("README.md"), "main moved\n").unwrap();
    git(&dir, &["commit", "-qam", "main moved"]);
    // What equipe wrote there before: a profile at the top, a decision, a critique, the tuner's signals.
    std::fs::write(dir.join("design/taste.md"), "# Taste\n\n## Style profile\n- Disliked: busy\n<!-- /style profile -->\n\n## Notes\n- calm\n").unwrap();
    std::fs::create_dir_all(dir.join("design/decisions")).unwrap();
    std::fs::write(dir.join("design/decisions/2026-11-17-login.md"), "## Login\n").unwrap();
    std::fs::write(dir.join("design/decisions/2026-11-17-login.png"), b"png").unwrap();
    std::fs::create_dir_all(dir.join("design/critiques")).unwrap();
    std::fs::write(dir.join("design/critiques/1794749173.md"), "## Whole\n").unwrap();
    std::fs::create_dir_all(dir.join("design/tuner")).unwrap();
    std::fs::write(dir.join("design/tuner/signals.jsonl"), "{\"kind\":\"like\",\"picture\":\"a.png\",\"liked\":true,\"weight\":50,\"date\":\"2026-11-17\"}\n").unwrap();
    // And the person's own file, which stays.
    std::fs::write(dir.join("prod-empty.light.1280.png"), b"png").unwrap();

    worktree::merge_into_main(&dir, "work", "equipe: merge T-1").unwrap();
    assert_eq!(status(&dir), "?? prod-empty.light.1280.png", "only the person's file is left");
    let taste = git(&dir, &["show", "main:design/taste.md"]);
    assert!(taste.contains("## Style profile") && taste.contains("the login screen: centred"), "both landed: {taste}");
    for file in ["design/decisions/2026-11-17-login.md", "design/decisions/2026-11-17-login.png", "design/critiques/1794749173.md"] {
        git(&dir, &["cat-file", "-e", &format!("main:{file}")]);
    }
    // The signals are the person's own on this machine: moved out of the tree, still read.
    assert!(!dir.join("design/tuner").exists());
    git(&dir, &["cat-file", "-e", "main:design/taste.md"]);
    assert!(git(&dir, &["ls-tree", "-r", "--name-only", "main"]).lines().all(|l| !l.contains("signals.jsonl")));
    let read = crate::taste::tuner::read_signals(&dir.join("design"));
    assert_eq!(read.len(), 1, "{read:?}");
    assert!(crate::taste::tuner::signals_file(&dir.join("design")).starts_with(dir.join(".git")));
    let subjects = git(&dir, &["log", "--format=%s", "main"]);
    assert!(subjects.lines().any(|s| s.starts_with("equipe: commit the design notes equipe wrote")), "{subjects}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signals_of_a_project_go_outside_the_working_tree() {
    let dir = repo("signals");
    let design = dir.join("design");
    crate::taste::tuner::append(&design, &[crate::taste::tuner::Record::Like { picture: "a.png".into(), liked: true, weight: 50, date: "2026-11-18".into() }]).unwrap();
    assert_eq!(status(&dir), "", "nothing in the checkout");
    assert_eq!(crate::taste::tuner::read_signals(&design).len(), 1);
    // A worktree of the same repository reads the same signals.
    let wt = dir.parent().unwrap().join(format!("equipe-own-signals-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt);
    git(&dir, &["worktree", "add", "-q", "-b", "work", &wt.to_string_lossy()]);
    assert_eq!(crate::taste::tuner::signals_file(&wt.join("design")), crate::taste::tuner::signals_file(&design));
    git(&dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    // A folder in no repository keeps them in it, as before.
    let loose = std::env::temp_dir().join(format!("equipe-own-loose-{}", std::process::id()));
    assert_eq!(crate::taste::tuner::signals_file(&loose), loose.join("tuner/signals.jsonl"));
    let _ = std::fs::remove_dir_all(&dir);
}
