//! How a squad's members did, in two windows of time side by side — before
//! and after a change to equipe — from the squad's own journal and its
//! members' Claude Code transcripts on this machine. It only reads, and
//! prints (or writes to `--out`) for the person running it: keep what it
//! says out of the repository. See docs/quality-gate.md.
//!
//! ```sh
//! scripts/quality-metrics.sh --squad acme --split 2026-11-05            # the week before and the week after
//! scripts/quality-metrics.sh --squad acme --a 2026-10-01..2026-10-15 --b 2026-10-15..2026-11-01
//! scripts/quality-metrics.sh --squad acme --versions 0.103.40 0.103.50  # by release commits' times
//! ```
//!
//! Dates are UTC: `2026-11-05`, `2026-11-05T09:30`, or unix seconds.

use std::path::PathBuf;

use equipe_core::quality::live::{self, Live, Window, date_of, median, ratio, unix_of};

const USAGE: &str = "usage: quality_metrics --squad <name> [--home <equipe home>] [--claude-projects <dir>] [--out <file>]
    (--split <date> [--days <n>] | --a <from>..<to> --b <from>..<to> | --versions <v1> <v2> [--repo <equipe checkout>])";

fn fail(why: &str) -> ! {
    eprintln!("{why}\n{USAGE}");
    std::process::exit(2)
}

fn window(text: &str) -> Window {
    let (from, to) = text.split_once("..").unwrap_or_else(|| fail(&format!("`{text}`: write a window as <from>..<to>")));
    let at = |t: &str| unix_of(t).unwrap_or_else(|| fail(&format!("`{t}` isn't a date")));
    Window { from: at(from), to: at(to) }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// When each release was cut, from the repository's "Release <version>" commits, oldest first.
fn releases(repo: &std::path::Path) -> Vec<(String, u64)> {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["log", "--format=%ct %s", "--grep=^Release "]).output();
    let Ok(out) = out else { return Vec::new() };
    let mut found: Vec<(String, u64)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (at, subject) = l.split_once(' ')?;
            Some((subject.strip_prefix("Release ")?.trim().to_owned(), at.parse().ok()?))
        })
        .collect();
    found.sort_by_key(|(_, at)| *at);
    found
}

/// The time a version ran: from its release to the next one's (or now).
fn version_window(releases: &[(String, u64)], version: &str) -> Window {
    let at = releases.iter().position(|(v, _)| v == version).unwrap_or_else(|| fail(&format!("no \"Release {version}\" commit in the repository (--repo)")));
    Window { from: releases[at].1, to: releases.get(at + 1).map(|(_, t)| *t).unwrap_or_else(now) }
}

fn pct(x: Option<f64>) -> String {
    x.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or_else(|| "–".into())
}

fn num(x: Option<f64>) -> String {
    x.map(|x| format!("{x:.2}")).unwrap_or_else(|| "–".into())
}

fn delta(a: Option<f64>, b: Option<f64>, points: bool) -> String {
    match (a, b) {
        (Some(a), Some(b)) if points => format!("{:+.0} pts", (b - a) * 100.0),
        (Some(a), Some(b)) => format!("{:+.2}", b - a),
        _ => String::new(),
    }
}

fn mean(xs: &[u64]) -> Option<f64> {
    (!xs.is_empty()).then(|| xs.iter().sum::<u64>() as f64 / xs.len() as f64)
}

fn when(w: &Window) -> String {
    format!("{} – {}", date_of(w.from), date_of(w.to))
}

fn table(a: &Live, b: &Live, wa: &Window, wb: &Window) -> String {
    let mut md = format!("| | A ({}) | B ({}) | change |\n|---|---|---|---|\n", when(wa), when(wb));
    let mut row = |name: &str, va: String, vb: String, d: String| md.push_str(&format!("| {name} | {va} | {vb} | {d} |\n"));
    row("tickets handed in", a.handed_in.to_string(), b.handed_in.to_string(), String::new());
    row("accepted the first time", format!("{} ({}/{})", pct(a.first_time_rate()), a.first_time, a.accepted), format!("{} ({}/{})", pct(b.first_time_rate()), b.first_time, b.accepted), delta(a.first_time_rate(), b.first_time_rate(), true));
    row("sent back at least once", format!("{} ({}/{})", pct(a.rework_rate()), a.reworked, a.handed_in), format!("{} ({}/{})", pct(b.rework_rate()), b.reworked, b.handed_in), delta(a.rework_rate(), b.rework_rate(), true));
    row("person stepping in per ticket", num(a.corrections_per_ticket()), num(b.corrections_per_ticket()), delta(a.corrections_per_ticket(), b.corrections_per_ticket(), false));
    row("  · reopens / comments / messages", format!("{} / {} / {}", a.person_reopens, a.person_comments, a.person_messages), format!("{} / {} / {}", b.person_reopens, b.person_comments, b.person_messages), String::new());
    let per = |l: &Live| (l.handed_in > 0).then(|| l.tool_calls as f64 / l.handed_in as f64);
    row("members' tool calls per ticket handed in", num(per(a)), num(per(b)), delta(per(a), per(b), false));
    let k = |x: Option<f64>| x.map(|x| format!("{:.0}k", x / 1000.0)).unwrap_or_else(|| "–".into());
    row("tokens per ticket (mean / median)", format!("{} / {}", k(mean(&a.tokens_per_ticket)), k(median(&a.tokens_per_ticket).map(|m| m as f64))), format!("{} / {}", k(mean(&b.tokens_per_ticket)), k(median(&b.tokens_per_ticket).map(|m| m as f64))), String::new());
    row("turns per ticket (mean)", num(mean(&a.turns_per_ticket)), num(mean(&b.turns_per_ticket)), delta(mean(&a.turns_per_ticket), mean(&b.turns_per_ticket), false));
    row("transcripts read", a.transcripts.to_string(), b.transcripts.to_string(), String::new());
    row("foreground sleeps attempted", a.rules.foreground_sleeps.to_string(), b.rules.foreground_sleeps.to_string(), String::new());
    row("`equipe browser` run", a.rules.equipe_browser.to_string(), b.rules.equipe_browser.to_string(), String::new());
    row("guard refusals", a.rules.guard_refusals.to_string(), b.rules.guard_refusals.to_string(), String::new());
    row("estimates", a.rules.estimates.to_string(), b.rules.estimates.to_string(), String::new());
    let rec = |l: &Live| ratio(l.rules.decisions_recommended, l.rules.decisions_with_options);
    row("ask_decision with a recommendation", format!("{} ({}/{})", pct(rec(a)), a.rules.decisions_recommended, a.rules.decisions_with_options), format!("{} ({}/{})", pct(rec(b)), b.rules.decisions_recommended, b.rules.decisions_with_options), delta(rec(a), rec(b), true));
    row("the engine's own question tool", a.rules.engine_questions.to_string(), b.rules.engine_questions.to_string(), String::new());
    row("numbered questions in prose", a.rules.prose_questions.to_string(), b.rules.prose_questions.to_string(), String::new());
    let mut skills: Vec<&String> = a.skills.keys().chain(b.skills.keys()).collect();
    skills.sort();
    skills.dedup();
    for s in skills {
        let (ka, na) = a.skills.get(s).copied().unwrap_or_default();
        let (kb, nb) = b.skills.get(s).copied().unwrap_or_default();
        row(&format!("skill `{s}` opened in time"), format!("{} ({ka}/{na})", pct(ratio(ka, na))), format!("{} ({kb}/{nb})", pct(ratio(kb, nb))), delta(ratio(ka, na), ratio(kb, nb), true));
    }
    let mut roles: Vec<&String> = a.by_role.keys().chain(b.by_role.keys()).collect();
    roles.sort();
    roles.dedup();
    for role in roles {
        let ra = a.by_role.get(role).cloned().unwrap_or_default();
        let rb = b.by_role.get(role).cloned().unwrap_or_default();
        row(&format!("{role}: accepted the first time"), format!("{} ({}/{})", pct(ratio(ra.first_time, ra.accepted)), ra.first_time, ra.accepted), format!("{} ({}/{})", pct(ratio(rb.first_time, rb.accepted)), rb.first_time, rb.accepted), delta(ratio(ra.first_time, ra.accepted), ratio(rb.first_time, rb.accepted), true));
    }
    md
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut squad, mut home, mut projects, mut out, mut repo) = (None, None, None, None, PathBuf::from("."));
    let (mut a, mut b, mut split, mut days, mut versions) = (None, None, None, 7u64, None);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(&format!("{arg} needs a value")));
        match arg.as_str() {
            "--squad" => squad = Some(value()),
            "--home" => home = Some(PathBuf::from(value())),
            "--claude-projects" => projects = Some(PathBuf::from(value())),
            "--out" => out = Some(PathBuf::from(value())),
            "--repo" => repo = PathBuf::from(value()),
            "--a" => a = Some(window(&value())),
            "--b" => b = Some(window(&value())),
            "--split" => split = Some(unix_of(&value()).unwrap_or_else(|| fail("--split takes a date"))),
            "--days" => days = value().parse().unwrap_or_else(|_| fail("--days takes a number")),
            "--versions" => versions = Some((value(), value())),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => fail(&format!("unknown argument {other}")),
        }
    }
    let squad = squad.unwrap_or_else(|| fail("--squad is needed"));
    let home = home.unwrap_or_else(equipe_providers::credentials::equipe_home);
    let projects = projects.or_else(equipe_core::quality::claude_projects).filter(|p| p.exists());
    let (wa, wb, said) = match (a, b, split, versions) {
        (Some(a), Some(b), None, None) => (a, b, "two windows".to_owned()),
        (None, None, Some(at), None) => (Window { from: at.saturating_sub(days * 86_400), to: at }, Window { from: at, to: (at + days * 86_400).min(now()) }, format!("{days} days before and after {}", date_of(at))),
        (None, None, None, Some((v1, v2))) => {
            let r = releases(&repo);
            (version_window(&r, &v1), version_window(&r, &v2), format!("equipe {v1} (A) and {v2} (B), by their release commits' times"))
        }
        _ => fail("give one of --split, --a with --b, or --versions"),
    };
    let db = equipe_core::journal::squad_dir(&home, &squad).join("squad.db");
    if !db.exists() {
        fail(&format!("no journal at {}", db.display()));
    }
    let squad_home = equipe_core::squad_home::path(&home, &squad);
    let measure = |w: Window| live::measure(&db, w, projects.as_deref(), Some(&squad_home)).unwrap_or_else(|e| fail(&format!("reading {}: {e}", db.display())));
    let (la, lb) = (measure(wa), measure(wb));
    let md = format!(
        "# {squad}: how members did, {said}\n\nFrom the squad's journal{}. Times are UTC.\n\n{}\nA ticket counts in the window it was first handed in; its acceptance, whenever it came. \"Stepping in\" counts the person's reopens, ticket comments and own messages to members other than the PM.\n",
        if projects.is_some() { " and its members' Claude Code transcripts" } else { "" },
        table(&la, &lb, &wa, &wb)
    );
    match out {
        Some(path) => {
            std::fs::write(&path, &md).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
            eprintln!("written to {}", path.display());
        }
        None => print!("{md}"),
    }
}
