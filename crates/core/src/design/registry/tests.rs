use super::*;

use std::process::Command;

fn merge(file: &str, base: &str, ours: &str, theirs: &str) -> std::result::Result<String, Vec<String>> {
    merge_text(file, Some(base), ours, theirs)
}

const FLOWS: &str = "flows/flows.json";

#[test]
fn registries_are_known_by_their_place_in_a_design_folder() {
    assert_eq!(registry_of("design/flows/flows.json"), Some(FLOWS));
    assert_eq!(registry_of("web/design/references/board.json"), Some("references/board.json"));
    assert_eq!(registry_of("design/directions/log.json"), Some("directions/log.json"));
    assert_eq!(registry_of("design/captures.json"), Some("captures.json"));
    assert_eq!(registry_of("flows/flows.json"), None, "only under a design folder");
    assert_eq!(registry_of("design/tokens.json"), None);
    assert_eq!(registry_of("mydesign/flows/flows.json"), None);
}

#[test]
fn two_flows_added_to_a_one_line_map_both_land_on_one_line() {
    let base = r#"{"flows":[{"id":"help","title":"Guide","screens":[{"id":"guide","states":[{"id":"default"}]}]}],"version":1}"#;
    let ours = r#"{"flows":[{"id":"help","title":"Guide","screens":[{"id":"guide","states":[{"id":"default"}]}]},{"id":"erp","title":"ERP"}],"version":1}"#;
    let theirs = r#"{"flows":[{"id":"help","title":"Guide","screens":[{"id":"guide","states":[{"id":"default"}]}]},{"id":"gallery","title":"갤러리"}],"version":1}"#;
    let merged = merge(FLOWS, base, ours, theirs).unwrap();
    assert_eq!(
        merged,
        r#"{"flows":[{"id":"help","title":"Guide","screens":[{"id":"guide","states":[{"id":"default"}]}]},{"id":"erp","title":"ERP"},{"id":"gallery","title":"갤러리"}],"version":1}"#,
        "main's first, then the branch's, keys in order, the text as written"
    );
    // The other way round, the same entries.
    let swapped = merge(FLOWS, base, theirs, ours).unwrap();
    assert!(swapped.contains(r#""id":"erp""#) && swapped.contains(r#""id":"gallery""#), "{swapped}");
}

#[test]
fn a_change_deep_in_one_flow_and_a_new_flow_merge_and_so_do_two_states() {
    // T-3427 changed two states of the ERP flow on main; the branch added a flow.
    let base = r#"{"flows":[{"id":"erp","screens":[{"id":"list","states":[{"id":"a","prep":[{"eval":"x()"}]},{"id":"b","captureVisible":false}]}]}]}"#;
    let ours = r#"{"flows":[{"id":"erp","screens":[{"id":"list","states":[{"id":"a","prep":[{"eval":"y()"}]},{"id":"b","captureVisible":true}]}]}]}"#;
    let theirs = r#"{"flows":[{"id":"erp","screens":[{"id":"list","states":[{"id":"a","prep":[{"eval":"x()"}]},{"id":"b","captureVisible":false}]}]},{"id":"gallery"}]}"#;
    let merged: Json = serde_json::from_str(&merge(FLOWS, base, ours, theirs).unwrap()).unwrap();
    let want: Json = serde_json::from_str(r#"{"flows":[{"id":"erp","screens":[{"id":"list","states":[{"id":"a","prep":[{"eval":"y()"}]},{"id":"b","captureVisible":true}]}]},{"id":"gallery"}]}"#).unwrap();
    assert_eq!(merged, want);
    // Each side adds a state to the same screen: both kept.
    let ours = base.replace(r#"{"id":"b","captureVisible":false}"#, r#"{"id":"b","captureVisible":false},{"id":"c"}"#);
    let theirs = base.replace(r#"{"id":"b","captureVisible":false}"#, r#"{"id":"b","captureVisible":false},{"id":"d"}"#);
    let merged = merge(FLOWS, base, &ours, &theirs).unwrap();
    assert!(merged.contains(r#"{"id":"b","captureVisible":false},{"id":"c"},{"id":"d"}"#), "{merged}");
}

#[test]
fn only_the_same_value_changed_two_ways_conflicts_and_it_is_named() {
    let base = r#"{"flows":[{"id":"erp","title":"ERP"},{"id":"help","title":"Help"}]}"#;
    let ours = r#"{"flows":[{"id":"erp","title":"ERP list"},{"id":"help","title":"Help"}]}"#;
    let theirs = r#"{"flows":[{"id":"erp","title":"ERP table"},{"id":"help","title":"Guide"}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap_err(), ["flows[id=erp].title"]);
    // The same change on both sides is no conflict.
    assert_eq!(merge(FLOWS, base, ours, ours).unwrap(), ours);
    // Both added an entry with the same id, differently: a conflict there.
    let ours = r#"{"flows":[{"id":"erp","title":"ERP"},{"id":"help","title":"Help"},{"id":"new","title":"A"}]}"#;
    let theirs = r#"{"flows":[{"id":"erp","title":"ERP"},{"id":"help","title":"Help"},{"id":"new","title":"B"}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap_err(), ["flows[id=new].title"]);
}

#[test]
fn an_entry_one_side_removed_goes_unless_the_other_changed_it() {
    let base = r#"{"flows":[{"id":"a","n":1},{"id":"b","n":1}]}"#;
    let ours = r#"{"flows":[{"id":"a","n":1}]}"#;
    let theirs = r#"{"flows":[{"id":"a","n":1},{"id":"b","n":1},{"id":"c","n":1}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap(), r#"{"flows":[{"id":"a","n":1},{"id":"c","n":1}]}"#);
    let theirs = r#"{"flows":[{"id":"a","n":1},{"id":"b","n":2}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap_err(), ["flows[id=b]"]);
}

#[test]
fn an_indented_board_keeps_its_indent_and_its_words() {
    let base = "[\n  {\n    \"id\": \"R1\",\n    \"learn\": \"동작·링크 합성\",\n    \"aspects\": [\n      \"pattern\"\n    ]\n  }\n]\n";
    let ours = "[\n  {\n    \"id\": \"R1\",\n    \"learn\": \"동작·링크 합성\",\n    \"aspects\": [\n      \"pattern\"\n    ]\n  },\n  {\n    \"id\": \"R2\",\n    \"learn\": \"a\",\n    \"aspects\": []\n  }\n]\n";
    let theirs = "[\n  {\n    \"id\": \"R1\",\n    \"learn\": \"동작·링크 합성\",\n    \"aspects\": [\n      \"pattern\"\n    ]\n  },\n  {\n    \"id\": \"R3\",\n    \"learn\": \"b\",\n    \"aspects\": []\n  }\n]\n";
    let merged = merge("references/board.json", base, ours, theirs).unwrap();
    assert_eq!(
        merged,
        "[\n  {\n    \"id\": \"R1\",\n    \"learn\": \"동작·링크 합성\",\n    \"aspects\": [\n      \"pattern\"\n    ]\n  },\n  {\n    \"id\": \"R2\",\n    \"learn\": \"a\",\n    \"aspects\": []\n  },\n  {\n    \"id\": \"R3\",\n    \"learn\": \"b\",\n    \"aspects\": []\n  }\n]\n"
    );
    assert_eq!(Style::of(&merged), Style { indent: Some("  ".into()), newline: true });
    assert_eq!(Style::of("{\"a\":1}"), Style { indent: None, newline: false });
}

#[test]
fn the_directions_log_is_merged_by_name() {
    let base = r#"[{"name":"t1-a","chosen":false}]"#;
    let ours = r#"[{"name":"t1-a","chosen":true}]"#;
    let theirs = r#"[{"name":"t1-a","chosen":false},{"name":"t2-a","chosen":false}]"#;
    assert_eq!(merge("directions/log.json", base, ours, theirs).unwrap(), r#"[{"name":"t1-a","chosen":true},{"name":"t2-a","chosen":false}]"#);
}

#[test]
fn lists_that_are_not_entries_keep_both_sides_additions() {
    let base = r#"{"flows":[{"id":"a","source":["x.tsx"]}]}"#;
    let ours = r#"{"flows":[{"id":"a","source":["x.tsx","y.tsx"]}]}"#;
    let theirs = r#"{"flows":[{"id":"a","source":["x.tsx","z.tsx"]}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap(), r#"{"flows":[{"id":"a","source":["x.tsx","y.tsx","z.tsx"]}]}"#);
    let theirs = r#"{"flows":[{"id":"a","source":["z.tsx"]}]}"#;
    assert_eq!(merge(FLOWS, base, ours, theirs).unwrap_err(), ["flows[id=a].source"]);
}

#[test]
fn the_captures_manifest_merges_as_its_own_and_a_file_that_is_not_json_is_refused() {
    let entry = |path: &str, hash: char| format!(r#"{{"path":"{path}","hash":"{}","size":1,"created_at":1}}"#, hash.to_string().repeat(40));
    let file = |entries: &[String]| format!("{{\"version\": 1, \"captures\": [\n{}\n]}}\n", entries.join(",\n"));
    let base = file(&[entry("flows/a/s/default.light.390.png", 'a')]);
    let ours = file(&[entry("flows/a/s/default.light.390.png", 'a'), entry("checks/o.png", 'b')]);
    let theirs = file(&[entry("flows/a/s/default.light.390.png", 'a'), entry("checks/t.png", 'c')]);
    let merged = merge("captures.json", &base, &ours, &theirs).unwrap();
    assert!(merged.contains("checks/o.png") && merged.contains("checks/t.png") && merged.lines().count() == 5, "{merged}");
    assert!(merge(FLOWS, "{}", "<<<<<<< ours", "{}").is_err());
}

#[test]
fn equipes_attributes_block_is_kept_once_with_the_persons_lines() {
    let once = with_attributes("*.png binary\n");
    assert!(once.starts_with("*.png binary\n") && once.contains("**/design/flows/flows.json merge=equipe-registry"), "{once}");
    assert_eq!(with_attributes(&once), once);
    assert!(driver_command(Path::new("/opt/it's/equipe")).starts_with("'/opt/it'\\''s/equipe' design merge-registry %O %A %B %P || git merge-file"));
}

#[test]
fn the_driver_writes_the_merge_over_ours_and_leaves_a_conflict_to_git() {
    let dir = std::env::temp_dir().join(format!("equipe-registry-driver-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (b, o, t) = (dir.join("b"), dir.join("o"), dir.join("t"));
    std::fs::write(&b, r#"{"flows":[{"id":"a"}]}"#).unwrap();
    std::fs::write(&o, r#"{"flows":[{"id":"a"},{"id":"m"}]}"#).unwrap();
    std::fs::write(&t, r#"{"flows":[{"id":"a"},{"id":"x"}]}"#).unwrap();
    run_driver(&b, &o, &t, "design/flows/flows.json").unwrap();
    assert_eq!(std::fs::read_to_string(&o).unwrap(), r#"{"flows":[{"id":"a"},{"id":"m"},{"id":"x"}]}"#);
    std::fs::write(&o, r#"{"flows":[{"id":"a","t":1}]}"#).unwrap();
    std::fs::write(&t, r#"{"flows":[{"id":"a","t":2}]}"#).unwrap();
    assert!(run_driver(&b, &o, &t, "design/flows/flows.json").unwrap_err().contains("flows[id=a].t"));
    assert_eq!(std::fs::read_to_string(&o).unwrap(), r#"{"flows":[{"id":"a","t":1}]}"#, "untouched: git marks it");
    assert!(run_driver(&b, &o, &t, "src/app.json").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── equipe's landing ──

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("design/flows")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("design/flows/flows.json"), r#"{"flows":[{"id":"help","title":"Help"}]}"#).unwrap();
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "base"]);
    dir
}

fn commit_on(dir: &Path, branch: &str, file: &str, text: &str) {
    git(dir, &["checkout", "-q", branch]);
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["commit", "-qam", file]);
}

#[test]
fn equipes_landing_merges_two_tickets_flows_and_tells_of_real_conflicts_only() {
    let dir = repo("land");
    git(&dir, &["branch", "work"]);
    git(&dir, &["branch", "other"]);
    commit_on(&dir, "work", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"gallery","title":"Gallery"}]}"#);
    commit_on(&dir, "other", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Guide"}]}"#);
    commit_on(&dir, "main", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"erp","title":"ERP"}]}"#);
    // Text-wise both branches conflict with main on the one line.
    assert_eq!(worktree::merge_conflicts(&dir, "work").unwrap(), Vec::<String>::new(), "the flows merge as JSON");
    worktree::merge_into_main(&dir, "work", "merge work").unwrap();
    let landed = git(&dir, &["show", "main:design/flows/flows.json"]);
    assert_eq!(landed, r#"{"flows":[{"id":"help","title":"Help"},{"id":"erp","title":"ERP"},{"id":"gallery","title":"Gallery"}]}"#);
    assert_eq!(std::fs::read_to_string(dir.join("design/flows/flows.json")).unwrap(), landed, "the checkout followed");
    // A title both changed — main kept Help, `other` made it Guide — merges too:
    // only `other` changed it since their common ancestor.
    assert_eq!(worktree::merge_conflicts(&dir, "other").unwrap(), Vec::<String>::new());
    // Changed two ways: a real conflict, told and nothing merged.
    git(&dir, &["branch", "third", "main"]);
    commit_on(&dir, "third", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"erp","title":"ERP list"},{"id":"gallery","title":"Gallery"}]}"#);
    commit_on(&dir, "main", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"erp","title":"ERP table"},{"id":"gallery","title":"Gallery"}]}"#);
    assert_eq!(worktree::merge_conflicts(&dir, "third").unwrap(), ["design/flows/flows.json"]);
    let before = git(&dir, &["rev-parse", "main"]);
    assert!(worktree::merge_into_main(&dir, "third", "merge third").unwrap_err().contains("conflicts with main in design/flows/flows.json"));
    assert_eq!(git(&dir, &["rev-parse", "main"]), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_registry_and_another_file_conflicting_is_still_a_conflict_in_that_file() {
    let dir = repo("mixed");
    git(&dir, &["branch", "work"]);
    commit_on(&dir, "work", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"a"}]}"#);
    commit_on(&dir, "work", "README.md", "work\n");
    commit_on(&dir, "main", "design/flows/flows.json", r#"{"flows":[{"id":"help","title":"Help"},{"id":"b"}]}"#);
    commit_on(&dir, "main", "README.md", "main\n");
    assert_eq!(worktree::merge_conflicts(&dir, "work").unwrap(), ["README.md"]);
    assert!(worktree::merge_into_main(&dir, "work", "m").unwrap_err().contains("README.md"));
    let _ = std::fs::remove_dir_all(&dir);
}
