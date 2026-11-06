//! The lint on screens that look generated and on clean ones, each rule's
//! own cases, waivers, the edit-time check and what a hand-in needs.

use std::path::{Path, PathBuf};

use super::*;

const TOKENS: &str = include_str!("fixtures/tokens.json");
const GENERATED_HTML: &str = include_str!("fixtures/generated.html");
const CLEAN_HTML: &str = include_str!("fixtures/clean.html");
const GENERATED_TSX: &str = include_str!("fixtures/generated.tsx");
const CLEAN_TSX: &str = include_str!("fixtures/clean.tsx");

/// A project with a design system and these files, in a folder of its own.
fn project(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("equipe-lint-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("design")).unwrap();
    std::fs::write(root.join("design/tokens.json"), TOKENS).unwrap();
    for (name, body) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    root
}

fn lint(root: &Path, file: &str) -> Report {
    let path = root.join(file);
    let design = design_dir_of(&path).expect("the design folder");
    let mut report = lint_sources(&[path], &System::read(&design));
    apply_waivers(&mut report, &design, None);
    report
}

fn rules_at(report: &Report, tier: Tier) -> Vec<String> {
    let mut v: Vec<String> = report.findings.iter().filter(|f| f.tier == tier && f.open()).map(|f| f.rule.clone()).collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn a_generated_looking_screen_fails_the_floor_and_shows_its_habits() {
    let root = project(&[("design/mockups/home.html", GENERATED_HTML)]);
    let r = lint(&root, "design/mockups/home.html");
    let floor = rules_at(&r, Tier::Floor);
    for rule in ["token-colour", "token-type", "token-space", "focus-visible", "reduced-motion", "invented-data", "control-name"] {
        assert!(floor.contains(&rule.to_owned()), "{rule} in {floor:?}\n{}", r.text());
    }
    let habits = rules_at(&r, Tier::Habit);
    for rule in ["eyebrow", "tracked-caps", "coloured-word", "gradient-hero", "nested-cards", "emoji-icons", "uniform-blocks", "decorative-dots", "pretend-submit"] {
        assert!(habits.contains(&rule.to_owned()), "{rule} in {habits:?}\n{}", r.text());
    }
    let s = r.summary();
    assert!(!s.passes() && s.floor.1 >= 6, "{s:?}");
    let invented: Vec<&str> = r.findings.iter().filter(|f| f.rule == "invented-data").map(|f| f.evidence.as_str()).collect();
    for what in ["trust claim", "lorem ipsum", "performance figure", "placeholder name", "testimonials"] {
        assert!(invented.iter().any(|e| e.contains(what)), "{what}: {invented:#?}");
    }
    assert!(r.text().contains("fix: "), "each rule says how to fix it");
}

#[test]
fn a_generated_looking_component_is_caught_in_jsx_too() {
    let root = project(&[("src/Pricing.tsx", GENERATED_TSX)]);
    let r = lint(&root, "src/Pricing.tsx");
    let floor = rules_at(&r, Tier::Floor);
    for rule in ["token-colour", "token-space", "focus-visible", "reduced-motion", "invented-data", "control-name"] {
        assert!(floor.contains(&rule.to_owned()), "{rule} in {floor:?}\n{}", r.text());
    }
    let habits = rules_at(&r, Tier::Habit);
    for rule in ["eyebrow", "tracked-caps", "coloured-word", "gradient-hero", "nested-cards", "emoji-icons", "pretend-submit"] {
        assert!(habits.contains(&rule.to_owned()), "{rule} in {habits:?}\n{}", r.text());
    }
    assert!(r.findings.iter().any(|f| f.rule == "token-colour" && f.evidence.contains("no `indigo` palette")));
}

#[test]
fn a_clean_screen_passes_the_floor_with_no_habits() {
    for (name, body) in [("design/mockups/settings.html", CLEAN_HTML), ("src/Members.tsx", CLEAN_TSX)] {
        let root = project(&[(name, body)]);
        let r = lint(&root, name);
        assert!(rules_at(&r, Tier::Floor).is_empty(), "{name}:\n{}", r.text());
        assert!(rules_at(&r, Tier::Habit).is_empty(), "{name}:\n{}", r.text());
        assert!(r.summary().passes());
    }
    // Literals the system holds are a note to name the token, never the floor.
    let root = project(&[("design/mockups/settings.html", CLEAN_HTML)]);
    let r = lint(&root, "design/mockups/settings.html");
    assert!(r.findings.iter().any(|f| f.rule == "literal-on-scale" && f.evidence.contains("gap: 16px is `space.4`")), "{}", r.text());
}

#[test]
fn colour_rules_know_the_systems_values_and_where_a_literal_is_only_a_fallback() {
    let css = ".a { color: #15161a; }\n.b { color: #123456; }\n.c { color: var(--x, #123456); }\n.d { background: white; }\n.e { border: 1px solid rgba(21, 22, 26, 1); }\n.f { color: currentColor; background: transparent; }";
    let root = project(&[("src/a.css", css)]);
    let r = lint(&root, "src/a.css");
    let at = |line: u32| r.findings.iter().filter(|f| f.line == Some(line)).map(|f| f.rule.as_str()).collect::<Vec<_>>();
    assert_eq!(at(1), vec!["literal-on-scale"], "the ink, written out");
    assert_eq!(at(2), vec!["token-colour"]);
    assert!(at(3).is_empty(), "a fallback");
    assert_eq!(at(4), vec!["literal-on-scale"], "white is the system's surface");
    assert_eq!(at(5), vec!["literal-on-scale"]);
    assert!(at(6).is_empty());
    // A shadow written out in the system's shadow colour names its token; another fails.
    let root = project(&[("src/b.css", ".a { box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12); }\n.b { box-shadow: 0 8px 24px rgba(99, 102, 241, 0.4); }")]);
    let r = lint(&root, "src/b.css");
    assert!(r.findings.iter().any(|f| f.line == Some(1) && f.rule == "literal-on-scale" && f.evidence.contains("shadow.raised")), "{}", r.text());
    assert!(r.findings.iter().any(|f| f.line == Some(2) && f.rule == "token-colour"));
    // A token file holds the values: nothing to say.
    let root = project(&[("design/build/css/tokens.css", ":root { --color-text: #15161A; --brand: #123456; }")]);
    assert!(lint(&root, "design/build/css/tokens.css").findings.is_empty());
}

#[test]
fn without_a_design_system_the_token_rules_say_they_didnt_run() {
    let dir = std::env::temp_dir().join(format!("equipe-lint-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.css"), ".a { color: #123456; padding: 13px }").unwrap();
    let r = lint_sources(&[dir.join("a.css")], &System::default());
    assert!(r.findings.iter().all(|f| !f.rule.starts_with("token-")), "{:#?}", r.findings);
    assert!(r.skipped.iter().any(|(id, why)| id == "token-colour" && why.contains("no design/tokens.json")));
    assert!(!r.checked.contains(&"token-colour".to_owned()));
    assert!(r.text().contains("token-colour not checked"));
}

#[test]
fn focus_motion_and_names_have_their_ways_out() {
    let ok = "<style>button:focus{outline:none} button:focus-visible{outline:2px solid var(--focus)} .s{animation:spin 1s} @media (prefers-reduced-motion: reduce){.s{animation:none}}</style>\n<button aria-label=\"Close\"><svg></svg></button><a href=\"/\"><img src=\"logo.svg\" alt=\"equipe\"></a><label>Name <input name=\"n\"></label><input type=\"hidden\" name=\"t\"><button class=\"outline-none focus-visible:ring-2\">Go</button>";
    let root = project(&[("design/a.html", ok)]);
    let r = lint(&root, "design/a.html");
    assert!(rules_at(&r, Tier::Floor).is_empty(), "{}", r.text());
    // Colour or fade alone moves nothing.
    let fade = ".a { transition: color 150ms, opacity 150ms; }";
    let root = project(&[("src/fade.css", fade)]);
    assert!(lint(&root, "src/fade.css").findings.iter().all(|f| f.rule != "reduced-motion"));
    let moves = ".a { transition: transform 150ms; }";
    let root = project(&[("src/moves.css", moves)]);
    assert!(lint(&root, "src/moves.css").findings.iter().any(|f| f.rule == "reduced-motion"));
}

#[test]
fn invented_proof_is_caught_but_real_figures_and_sourced_ones_are_not() {
    for claim in ["Trusted by 2,000 teams", "Join 10k+ creators today", "Rated 4.9/5 by our customers", "Over 1M+ downloads", "★★★★★ loved it", "Built for 99.99% uptime", "Lorem ipsum dolor"] {
        assert!(scan::invented_claim(claim).is_some(), "{claim}");
    }
    for real in ["1,204 users", "3 members", "Saved 09:12", "Invoice #10023", "4 of 12 done", "Uptime 99.98%"] {
        assert!(scan::invented_claim(real).is_none(), "{real}: {:?}", scan::invented_claim(real));
    }
    let sourced = "<section data-source=\"analytics, Oct 2026\"><p>Used by 120 teams</p></section>\n<!-- source: the CRM export of 2026-10-30 -->\n<p>Trusted by 2,000 teams</p>";
    let root = project(&[("design/a.html", sourced)]);
    assert!(lint(&root, "design/a.html").findings.iter().all(|f| f.rule != "invented-data"));
}

#[test]
fn a_dot_beside_a_real_status_is_not_decorative_and_one_label_over_a_heading_is_fine_among_many() {
    let html = "<li><span class=\"status-dot\" role=\"status\" aria-label=\"Online\"></span> Online</li>\n<h2>Team</h2><p>Who works here.</p><p class=\"label\">Billing</p><h2>Plan</h2><h2>Invoices</h2><h2>Danger zone</h2>";
    let root = project(&[("design/a.html", html)]);
    let r = lint(&root, "design/a.html");
    assert!(rules_at(&r, Tier::Habit).is_empty(), "{}", r.text());
}

#[test]
fn a_habit_is_waived_with_its_reason_and_the_floor_is_not() {
    let root = project(&[
        ("design/mockups/home.html", GENERATED_HTML),
        ("design/decisions/2026-11-01-labels.md", "# Labels in capitals?\nChose: keep\nWhy: the brand's labels\n\nWaive tracked-caps: the brand sets its section labels in capitals (brand.md, Locked)\nWaive contrast: it looks calmer\nWaive token-colour: we like these purples\n"),
    ]);
    let r = lint(&root, "design/mockups/home.html");
    let caps: Vec<&Finding> = r.findings.iter().filter(|f| f.rule == "tracked-caps").collect();
    assert!(!caps.is_empty() && caps.iter().all(|f| f.waived.as_deref() == Some("the brand sets its section labels in capitals (brand.md, Locked)")));
    assert!(r.findings.iter().filter(|f| f.rule == "token-colour").all(Finding::open), "a floor rule is never waived");
    assert!(r.refused.iter().any(|x| x.contains("`token-colour` is a floor rule")), "{:?}", r.refused);
    let s = r.summary();
    assert!(s.waived.iter().any(|(id, why)| id == "tracked-caps" && why.contains("brand")));
    assert!(!s.open.iter().any(|(id, _)| id == "tracked-caps"));
    assert!(!s.passes());
    assert!(r.text().contains("kept: the brand sets"));
}

#[test]
fn the_summary_line_reads_back_from_a_runs_output() {
    let root = project(&[("design/mockups/home.html", GENERATED_HTML)]);
    let mut s = lint(&root, "design/mockups/home.html").summary();
    s.screen = Some("marketing/home".into());
    let output = format!("lots of lines\n{}\n", mark_line(&s));
    assert_eq!(summary_in(&output), Some(s));
    assert_eq!(summary_in("nothing here"), None);
}

#[test]
fn only_the_floor_when_asked() {
    let root = project(&[("design/mockups/home.html", GENERATED_HTML)]);
    let mut r = lint(&root, "design/mockups/home.html");
    r.only(Tier::Floor);
    assert!(r.findings.iter().all(|f| f.tier == Tier::Floor) && !r.findings.is_empty());
    assert!(r.checked.iter().all(|id| rules::rule(id).is_some_and(|x| x.tier == Tier::Floor)));
}

#[test]
fn the_edit_time_check_hands_back_the_floor_failures_in_what_was_just_written() {
    let before = "<main>\n  <p style=\"color: #123456\">Old, not yours</p>\n</main>\n";
    let root = project(&[("design/mockups/a.html", before)]);
    let file = root.join("design/mockups/a.html");
    let written = "  <button></button>\n  <p style=\"padding: 13px\">Trusted by 5,000 teams</p>\n";
    std::fs::write(&file, format!("{before}{written}")).unwrap();
    let input = serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": file, "old_string": "x", "new_string": written }, "tool_response": {} });
    let said = hook::run(&input, true).expect("the floor fails");
    assert!(said.contains("control-name") && said.contains("token-space") && said.contains("invented-data"), "{said}");
    assert!(!said.contains("#123456"), "what was there before isn't put on the edit: {said}");
    assert!(said.contains("Fix: "));
    // Written down to check before a hand-in.
    let design = root.join("design");
    let pending = record::read_pending(&std::fs::canonicalize(&design).unwrap());
    assert_eq!(pending.edited.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), vec!["design/mockups/a.html"]);
    // A clean edit says nothing; a file that isn't UI isn't looked at.
    let clean = root.join("design/mockups/b.html");
    std::fs::write(&clean, CLEAN_HTML).unwrap();
    let input = serde_json::json!({ "tool_name": "Write", "tool_input": { "file_path": clean, "content": CLEAN_HTML }, "tool_response": { "type": "create" } });
    assert_eq!(hook::run(&input, false), None);
    let input = serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": root.join("README.md"), "new_string": "x" } });
    assert_eq!(hook::run(&input, true), None);
}

fn verdict(pass: bool, at: u64, first: bool) -> record::Verdict {
    record::Verdict { pass, fixes: if pass { vec![] } else { vec![record::Fix { place: "hero".into(), what: "the headline wraps badly at 360".into(), why: String::new() }] }, summary: "x".into(), at, by: "claude".into(), widths: vec![360, 1280], first, ..Default::default() }
}

#[test]
fn directions_explored_are_left_to_the_direction_log_and_only_shipping_files_wait_for_a_check() {
    let root = project(&[("design/directions/ledger/index.html", CLEAN_HTML), ("design/directions/ledger/styles.css", "a {}"), ("index.html", CLEAN_HTML)]);
    let design = std::fs::canonicalize(root.join("design")).unwrap();
    for f in ["design/directions/ledger/index.html", "design/directions/ledger/styles.css"] {
        record::note_edit(&design, &root.join(f), true).unwrap();
    }
    assert!(record::hand_in(&design).is_empty(), "an explored direction isn't what ships: {:?}", record::hand_in(&design));
    record::note_edit(&design, &root.join("index.html"), true).unwrap();
    let said = record::hand_in(&design);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("index.html") && said[0].contains("`equipe design lint index.html --render --screen index`") && said[0].contains("`equipe design second-look index`"), "the file, the rule and the exact commands: {said:?}");
}

#[test]
fn a_check_kept_by_another_design_folder_still_clears_the_edit_where_the_hand_in_looks() {
    let root = project(&[("site/index.html", CLEAN_HTML), ("design/directions/a/tokens.json", TOKENS)]);
    let design = std::fs::canonicalize(root.join("design")).unwrap();
    let file = std::fs::canonicalize(root.join("site/index.html")).unwrap();
    record::note_edit(&design, &file, true).unwrap();
    // Checked against another system (`--design design/directions/a`): the
    // check is kept where the hand-in looks all the same.
    let other = design.join("directions/a");
    assert_eq!(record::ledger_home(&other, std::slice::from_ref(&file)), design);
    assert_eq!(record::ledger_home(&other, &[]), design, "a design folder inside another keeps its checks in the outer one");
    let lone = project(&[]);
    let lone = std::fs::canonicalize(lone.join("design")).unwrap();
    assert_eq!(record::ledger_home(&lone, &[]), lone);
    // A check an older equipe kept in the other folder is named in the refusal.
    let mut report = Report { widths: vec![360], ..Report::default() };
    report.checked.push("contrast".into());
    let rec = record::of(&other, "landing/home", &report, std::slice::from_ref(&file), None, true);
    record::write(&other, &rec).unwrap();
    let said = record::hand_in(&design);
    assert!(said.len() == 1 && said[0].contains("design/directions/a/checks/") && said[0].contains("--screen landing/home"), "{said:?}");
    assert_eq!(record::record_home(&other, "landing/home"), other, "a second look finds the record where it is");
    assert_eq!(record::record_home(&other, "nowhere"), design);
}

#[test]
fn a_designer_hands_in_only_when_each_edit_was_drawn_passed_the_floor_and_had_a_second_look() {
    let root = project(&[("design/mockups/home.html", CLEAN_HTML)]);
    let design = std::fs::canonicalize(root.join("design")).unwrap();
    let file = design.join("mockups/home.html");
    assert!(record::hand_in(&design).is_empty(), "nothing edited, nothing to show");
    record::note_edit(&design, &file, true).unwrap();
    let said = record::hand_in(&design);
    assert!(said.len() == 1 && said[0].contains("was edited and not checked"), "{said:?}");
    // Checked, drawn: the edit is cleared, the screen waits for its looks.
    let mut report = lint_sources(std::slice::from_ref(&file), &System::read(&design));
    report.widths = vec![360, 1280];
    let new = record::note_check(&design, "settings/notifications", std::slice::from_ref(&file)).unwrap();
    assert!(new);
    let mut rec = record::of(&design, "settings/notifications", &report, std::slice::from_ref(&file), None, new);
    record::write(&design, &rec).unwrap();
    let said = record::hand_in(&design);
    assert!(said.iter().any(|s| s.contains("first screenful")) && said.iter().any(|s| s.contains("no second look")), "{said:?}");
    // A look that asks for fixes isn't a pass.
    rec.first = Some(verdict(true, rec.at, true));
    rec.verdict = Some(verdict(false, rec.at, false));
    record::write(&design, &rec).unwrap();
    assert!(record::hand_in(&design).iter().any(|s| s.contains("asked for 1 fix")));
    // Passed after the check: nothing stands in the way.
    rec.verdict = Some(verdict(true, rec.at, false));
    record::write(&design, &rec).unwrap();
    assert!(record::hand_in(&design).is_empty(), "{:?}", record::hand_in(&design));
    // A look older than the check doesn't count.
    rec.verdict = Some(verdict(true, rec.at - 10, false));
    record::write(&design, &rec).unwrap();
    assert!(record::hand_in(&design).iter().any(|s| s.contains("changed after its second look")));
    // A floor failure blocks whatever the look said.
    rec.verdict = Some(verdict(true, rec.at, false));
    rec.summary.floor = (5, 6);
    rec.summary.failing = vec![("contrast".into(), "p.hint".into())];
    record::write(&design, &rec).unwrap();
    assert!(record::hand_in(&design).iter().any(|s| s.contains("fails the floor (contrast)") && s.contains("never waived")));
    record::handed_in(&design);
    assert!(record::hand_in(&design).is_empty());
}

#[test]
fn a_records_verdict_survives_a_rerun_on_unchanged_files_but_not_a_change() {
    let root = project(&[("design/mockups/home.html", CLEAN_HTML)]);
    let design = std::fs::canonicalize(root.join("design")).unwrap();
    let file = design.join("mockups/home.html");
    let mut report = Report { widths: vec![390], ..Report::default() };
    report.checked.push("contrast".into());
    let mut rec = record::of(&design, "a/b", &report, std::slice::from_ref(&file), None, false);
    rec.verdict = Some(verdict(true, rec.at, false));
    let again = record::of(&design, "a/b", &report, std::slice::from_ref(&file), Some(rec.clone()), false);
    assert!(again.verdict.is_some());
    let mut changed = rec.clone();
    changed.files[0].modified -= 5;
    assert!(record::of(&design, "a/b", &report, std::slice::from_ref(&file), Some(changed), false).verdict.is_none());
    assert_eq!(record::safe("../a/b c/../d"), "a/b-c/d");
}
