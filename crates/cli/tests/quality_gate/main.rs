//! The behaviour eval of equipe's quality gate (docs/quality-gate.md): real
//! members, run through equipe's own pipeline, on small fixed tasks, graded
//! automatically — once per variant of equipe, so a change to charters,
//! skills, tool descriptions, hooks or context handling shows what it does
//! to how members work.
//!
//! Each run starts a real `equipe daemon run` (the variant's binary) in a
//! throwaway home under /tmp, makes the squad on disk as the onboarding does,
//! adds the PM and the member under test over the daemon's socket as a
//! screen's `/add` does, and puts the task to them as the person would — a
//! ticket on the board, or a message. So the member gets what it gets in
//! production: its charter, its skills, equipe's guard hook, the board's MCP
//! tools, the engine flags, permissions and fence the runtime sets. An
//! automatic person allows what members ask permission for, answers their
//! questions with the recommended option, and declines team changes.
//!
//! It spends tokens on the person's own engine login, so it is ignored by
//! default. Run it with `scripts/quality-gate.sh`, or:
//!
//! ```sh
//! EQUIPE_EVAL_TASKS=backend-fix-failing-test EQUIPE_EVAL_RUNS=1 \
//!   cargo test -p equipe --test quality_gate -- --ignored --nocapture
//! ```
//!
//! - `EQUIPE_EVAL_A`: the baseline's `equipe` binary (left out: B alone, no verdict)
//! - `EQUIPE_EVAL_B`: the candidate's (default: this build's)
//! - `EQUIPE_EVAL_TASKS`: task names, comma-separated (default: all)
//! - `EQUIPE_EVAL_RUNS`: runs of each task per variant (default 3)
//! - `EQUIPE_EVAL_JOBS`: runs at once (default 1)
//! - `EQUIPE_EVAL_THRESHOLD`: percentage points a compliance rate may drop (default 10)
//! - `EQUIPE_EVAL_ENGINE`: `claude` (default) or `codex`; `default` for each role's own
//! - `EQUIPE_EVAL_MODEL`: one model for every member (default: each role's own)
//! - `EQUIPE_EVAL_OUT`: where the report goes (default `target/quality-gate/<time>`)
//! - `EQUIPE_EVAL_KEEP=1`: keep each run's home and Claude Code's transcript folders
//! - `EQUIPE_EVAL_REPORT=<runs.jsonl>[,<more.jsonl>]`: only write the report again from runs kept

#![cfg(unix)]

mod harness;
mod tasks;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use equipe_core::protocol::{Engine, Event, SessionId};
use equipe_core::quality::{self, Rules, Step, Tokens};
use serde::{Deserialize, Serialize};
use serde_json::json;

use harness::{Outcome, Run, Setup, State, Variant};
use tasks::{Check, TASKS, Task};

/// One run, as the report keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub task: String,
    pub role: String,
    pub variant: String,
    pub attempt: u32,
    pub pass: bool,
    pub checks: Vec<CheckRecord>,
    /// Each rule metric: kept (`true`), broken (`false`), or not in play (`null`).
    pub compliance: BTreeMap<String, Option<bool>>,
    /// Each skill the situation called for, and when it was opened.
    pub skills: Vec<(String, String)>,
    /// Tokens of the member under test, and of the whole run (the PM's too).
    pub tokens: TokenRecord,
    pub tokens_all: TokenRecord,
    /// What Claude Code put on the run at API prices (a subscription isn't charged it).
    pub cost_usd: f64,
    pub wall_secs: f64,
    pub tool_calls: usize,
    pub transcripts: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckRecord {
    pub name: String,
    pub ok: bool,
    pub required: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRecord {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub total: u64,
}

impl From<Tokens> for TokenRecord {
    fn from(t: Tokens) -> Self {
        Self { input: t.input, cache_write: t.cache_write, cache_read: t.cache_read, output: t.output, total: t.total() }
    }
}

impl From<&Check> for CheckRecord {
    fn from(c: &Check) -> Self {
        Self { name: c.name.to_owned(), ok: c.ok, required: c.required, detail: c.detail.clone() }
    }
}

/// The rule metrics, in the report's order.
pub const METRICS: &[&str] = &[
    "no foreground sleep",
    "no equipe browser",
    "no guard refusal",
    "estimate on ticket work",
    "hand-in through the board",
    "ask_decision marks a recommendation",
    "no engine question tool",
    "no numbered questions in prose",
    "skills opened in time",
];

/// A member's steps from the daemon's events, when there is no transcript
/// to read (Codex): each tool's name and what it was used on — enough for
/// the rules a command's words show, not for a tool's whole input.
pub fn steps_from_events(events: &[(f64, Event)], session: SessionId) -> Vec<Step> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            Event::ToolUse { session: s, tool, target, background, .. } if *s == session => Some(Step::Tool {
                id: String::new(),
                name: tool.clone(),
                input: json!({ "command": target.clone().unwrap_or_default(), "file_path": target.clone().unwrap_or_default(), "run_in_background": background }),
            }),
            Event::TextDelta { session: s, text } if *s == session => Some(Step::Reply(text.clone())),
            _ => None,
        })
        .collect()
}

/// How long ticket work must take before `estimate` is expected: "more than a couple of minutes".
pub const ESTIMATE_AFTER_SECS: f64 = 180.0;

/// How long `session` worked on tickets: from each `WorkStarted` to its
/// `WorkEnded` (or the last event).
pub fn work_seconds(events: &[(f64, Event)], session: SessionId) -> f64 {
    let mut total = 0.0;
    let mut since: Option<f64> = None;
    for (at, e) in events {
        match e {
            Event::WorkStarted { session: s, .. } if *s == session => {
                since.get_or_insert(*at);
            }
            Event::WorkEnded { session: s, .. } if *s == session => {
                if let Some(start) = since.take() {
                    total += at - start;
                }
            }
            _ => {}
        }
    }
    if let (Some(start), Some((last, _))) = (since, events.last()) {
        total += last - start;
    }
    total
}

/// The rules and skills a run of `task` shows of its member.
pub fn compliance(task: &Task, out: &Outcome) -> (BTreeMap<String, Option<bool>>, Vec<quality::SkillCheck>, Rules) {
    let transcripts = out.transcripts_of(task.role);
    let mut rules = Rules::default();
    let mut skills = Vec::new();
    if transcripts.is_empty() {
        if let Some(session) = out.state.session(task.role) {
            rules = quality::rules(&steps_from_events(&out.state.events, session));
        }
    }
    for t in &transcripts {
        rules.add(&quality::rules(&t.steps));
        if !t.sidechain {
            skills.extend(quality::skill_checks(t, Some(task.role)));
        }
    }
    let asked: Vec<_> = out.state.person.questions.iter().filter(|q| q.member == task.role || q.member.starts_with(&format!("{}-", task.role))).collect();
    let with_options: Vec<_> = asked.iter().filter(|q| q.questions.iter().any(|x| !x.options.is_empty())).collect();
    let recommended = with_options.iter().filter(|q| q.questions.iter().filter(|x| !x.options.is_empty()).all(|x| x.options.iter().any(|o| o.shown().1))).count();
    let worked_ticket = task.role != "pm" && out.state.tickets_of(task.role).next().is_some();
    let acted = rules.tool_calls > 0;
    let mut m = BTreeMap::new();
    let mut put = |name: &str, value: Option<bool>| {
        m.insert(name.to_owned(), value);
    };
    put("no foreground sleep", acted.then_some(rules.foreground_sleeps == 0));
    put("no equipe browser", acted.then_some(rules.equipe_browser == 0));
    put("no guard refusal", acted.then_some(rules.guard_refusals == 0));
    // The rule is for work of more than a couple of minutes.
    let long_work = out.state.session(task.role).is_some_and(|s| work_seconds(&out.state.events, s) >= ESTIMATE_AFTER_SECS);
    put("estimate on ticket work", (worked_ticket && long_work).then_some(rules.estimates > 0));
    put("hand-in through the board", worked_ticket.then_some(rules.submits > 0));
    put("ask_decision marks a recommendation", (!with_options.is_empty()).then_some(recommended == with_options.len()));
    put("no engine question tool", acted.then_some(rules.engine_questions == 0));
    put("no numbered questions in prose", acted.then_some(rules.prose_questions == 0));
    put("skills opened in time", (!skills.is_empty()).then(|| skills.iter().all(|c| c.opened.in_time())));
    (m, skills, rules)
}

fn record(task: &Task, variant: &Variant, attempt: u32, checks: &[Check], out: &Outcome) -> Record {
    let (compliance, skills, rules) = compliance(task, out);
    let mut mine = Tokens::default();
    let mut all = Tokens::default();
    for t in &out.transcripts {
        all.add(t.tokens);
        if t.role.as_deref() == Some(task.role) {
            mine.add(t.tokens);
        }
    }
    // No transcripts (Codex): the turns' own usage, as the daemon told it.
    if out.transcripts.is_empty() {
        for (session, usage) in &out.state.usage {
            let t = usage.iter().fold(Tokens::default(), |mut t, u| {
                t.add(Tokens { input: u.input_tokens.unwrap_or(0), cache_read: u.cache_tokens.unwrap_or(0), output: u.output_tokens.unwrap_or(0), cache_write: 0 });
                t
            });
            all.add(t);
            if out.state.session(task.role) == Some(*session) {
                mine.add(t);
            }
        }
    }
    Record {
        task: task.name.into(),
        role: task.role.into(),
        variant: variant.name.clone(),
        attempt,
        pass: checks.iter().all(|c| c.ok || !c.required),
        checks: checks.iter().map(CheckRecord::from).collect(),
        compliance,
        skills: skills.iter().map(|c| (c.skill.clone(), format!("{:?}", c.opened))).collect(),
        tokens: mine.into(),
        tokens_all: all.into(),
        cost_usd: out.cost_usd(),
        wall_secs: out.wall.as_secs_f64(),
        tool_calls: rules.tool_calls,
        transcripts: out.transcripts.len(),
        errors: out.state.errors.clone(),
    }
}

/// One run of `task` on `variant`, graded.
async fn run_once(task: Task, variant: Variant, attempt: u32, setup: Setup, out: PathBuf) -> Record {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tag = format!("eqeval-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let root = harness::root_for(&tag);
    let fx = tasks::setup(&task, &root);
    let projects: Vec<PathBuf> = fx.repo.iter().cloned().collect();
    let mut run = Run::start(&variant, &root, &tag, &projects, setup.clone()).await;
    tasks::play(&task, &mut run, &fx).await;
    let dir = out.join("runs").join(format!("{}-{}{}", task.name, variant.name, attempt));
    let outcome = run.finish(&dir).await;
    let checks = tasks::grade(&task, &fx, &outcome);
    let rec = record(&task, &variant, attempt, &checks, &outcome);
    std::fs::write(dir.join("record.json"), serde_json::to_string_pretty(&rec).unwrap()).unwrap();
    if !setup.keep {
        // The run's worktrees were made from its fixture repositories, under the root too.
        let _ = std::fs::remove_dir_all(&root);
    }
    rec
}

/// A run that broke off before it could be graded: failed, with why.
fn broken(task: &Task, variant: &str, attempt: u32, why: &str) -> Record {
    Record {
        task: task.name.into(),
        role: task.role.into(),
        variant: variant.into(),
        attempt,
        pass: false,
        checks: vec![CheckRecord { name: "the run finished".into(), ok: false, required: true, detail: why.into() }],
        compliance: BTreeMap::new(),
        skills: vec![],
        tokens: TokenRecord::default(),
        tokens_all: TokenRecord::default(),
        cost_usd: 0.0,
        wall_secs: 0.0,
        tool_calls: 0,
        transcripts: 0,
        errors: vec![why.into()],
    }
}

/// What changed from A to B that the gate flags.
#[derive(Debug, Clone, PartialEq)]
pub enum Flag {
    /// A task passed less often.
    Task { task: String, a: f64, b: f64 },
    /// A rule was kept less often, by more than the threshold.
    Metric { metric: String, a: f64, b: f64 },
}

/// The share of `records` of `variant` (and `task`, when given) that passed.
fn pass_rate(records: &[Record], variant: &str, task: Option<&str>) -> Option<f64> {
    let runs: Vec<&Record> = records.iter().filter(|r| r.variant == variant && task.is_none_or(|t| r.task == t)).collect();
    (!runs.is_empty()).then(|| runs.iter().filter(|r| r.pass).count() as f64 / runs.len() as f64)
}

/// How often `metric` was kept where it was in play, across `variant`'s runs: (kept, in play).
fn metric_rate(records: &[Record], variant: &str, metric: &str) -> (usize, usize) {
    let seen: Vec<bool> = records.iter().filter(|r| r.variant == variant).filter_map(|r| r.compliance.get(metric).copied().flatten()).collect();
    (seen.iter().filter(|k| **k).count(), seen.len())
}

/// The gate's verdict: each task B passes less often than A, and each rule
/// B keeps less often by more than `threshold` percentage points. Empty: no
/// regression found.
pub fn verdict(records: &[Record], threshold: f64) -> Vec<Flag> {
    let mut flags = Vec::new();
    let mut names: Vec<&str> = records.iter().map(|r| r.task.as_str()).collect();
    names.dedup();
    names.sort();
    names.dedup();
    for task in names {
        if let (Some(a), Some(b)) = (pass_rate(records, "A", Some(task)), pass_rate(records, "B", Some(task)))
            && b + 1e-9 < a
        {
            flags.push(Flag::Task { task: task.to_owned(), a, b });
        }
    }
    for metric in METRICS {
        let (ka, na) = metric_rate(records, "A", metric);
        let (kb, nb) = metric_rate(records, "B", metric);
        if na == 0 || nb == 0 {
            continue;
        }
        let (a, b) = (ka as f64 / na as f64, kb as f64 / nb as f64);
        if (a - b) * 100.0 > threshold + 1e-9 {
            flags.push(Flag::Metric { metric: (*metric).to_owned(), a, b });
        }
    }
    flags
}

fn pct(x: Option<f64>) -> String {
    x.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or_else(|| "–".into())
}

fn mean(xs: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = xs.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// The report, in Markdown.
pub fn report(records: &[Record], threshold: f64, about: &str) -> String {
    let variants: Vec<&str> = ["A", "B"].into_iter().filter(|v| records.iter().any(|r| r.variant == *v)).collect();
    let mut md = String::from("# equipe quality gate\n\n");
    md.push_str(about);
    md.push('\n');
    let flags = verdict(records, threshold);
    if variants.len() == 2 {
        if flags.is_empty() {
            md.push_str(&format!("**Verdict: PASS** — no task passed less often on B, and no rule was kept less often by more than {threshold} points.\n\n"));
        } else {
            md.push_str("**Verdict: FAIL** — B is worse than A here:\n\n");
            for f in &flags {
                match f {
                    Flag::Task { task, a, b } => md.push_str(&format!("- task `{task}`: passed {} on A, {} on B\n", pct(Some(*a)), pct(Some(*b)))),
                    Flag::Metric { metric, a, b } => md.push_str(&format!("- rule \"{metric}\": kept {} on A, {} on B (more than {threshold} points down)\n", pct(Some(*a)), pct(Some(*b)))),
                }
            }
            md.push('\n');
        }
    } else {
        md.push_str("Only one variant ran: no verdict.\n\n");
    }
    md.push_str("## Tasks\n\n| task | role |");
    for v in &variants {
        md.push_str(&format!(" {v} pass | {v} tokens (member / run) | {v} cost | {v} minutes |"));
    }
    md.push_str("\n|---|---|");
    for _ in &variants {
        md.push_str("---|---|---|---|");
    }
    md.push('\n');
    let mut names: Vec<(String, String)> = records.iter().map(|r| (r.task.clone(), r.role.clone())).collect();
    names.sort();
    names.dedup();
    for (task, role) in &names {
        md.push_str(&format!("| `{task}` | {role} |"));
        for v in &variants {
            let runs: Vec<&Record> = records.iter().filter(|r| r.variant == *v && r.task == *task).collect();
            let rate = pass_rate(records, v, Some(task));
            let member = mean(runs.iter().map(|r| r.tokens.total as f64));
            let all = mean(runs.iter().map(|r| r.tokens_all.total as f64));
            let cost = mean(runs.iter().map(|r| r.cost_usd));
            let minutes = mean(runs.iter().map(|r| r.wall_secs / 60.0));
            let k = |x: Option<f64>| x.map(|x| format!("{:.0}k", x / 1000.0)).unwrap_or_else(|| "–".into());
            md.push_str(&format!(
                " {} ({}/{}) | {} / {} | {} | {} |",
                pct(rate),
                runs.iter().filter(|r| r.pass).count(),
                runs.len(),
                k(member),
                k(all),
                cost.map(|c| format!("${c:.2}")).unwrap_or_else(|| "–".into()),
                minutes.map(|m| format!("{m:.1}")).unwrap_or_else(|| "–".into())
            ));
        }
        md.push('\n');
    }
    md.push_str("\nTokens are input + cache writes + cache reads + output, from Claude Code's transcripts; cost is what Claude Code puts on them at API prices, means per run.\n\n## Rules\n\n| rule |");
    for v in &variants {
        md.push_str(&format!(" {v} kept |"));
    }
    md.push_str("\n|---|");
    for _ in &variants {
        md.push_str("---|");
    }
    md.push('\n');
    for metric in METRICS {
        md.push_str(&format!("| {metric} |"));
        for v in &variants {
            let (k, n) = metric_rate(records, v, metric);
            md.push_str(&format!(" {} ({k}/{n}) |", pct((n > 0).then(|| k as f64 / n as f64))));
        }
        md.push('\n');
    }
    md.push_str("\nA rule counts only in runs where it was in play: an estimate where a member worked a ticket, a recommendation where it asked with options.\n\n## Runs that failed\n\n");
    let failed: Vec<&Record> = records.iter().filter(|r| !r.pass).collect();
    if failed.is_empty() {
        md.push_str("None.\n");
    }
    for r in failed {
        let why: Vec<String> = r.checks.iter().filter(|c| c.required && !c.ok).map(|c| if c.detail.is_empty() { c.name.clone() } else { format!("{} ({})", c.name, c.detail) }).collect();
        md.push_str(&format!("- `{}` {}{}: {}\n", r.task, r.variant, r.attempt, why.join("; ")));
    }
    let skills: Vec<String> = records.iter().flat_map(|r| r.skills.iter().map(move |(s, o)| format!("{} {s} {o}", r.variant))).collect();
    if !skills.is_empty() {
        md.push_str("\n## Skills the situations called for\n\n");
        let mut counted: BTreeMap<String, usize> = BTreeMap::new();
        for s in skills {
            *counted.entry(s).or_default() += 1;
        }
        for (s, n) in counted {
            md.push_str(&format!("- {s}: {n}\n"));
        }
    }
    let total: f64 = records.iter().map(|r| r.cost_usd).sum();
    let tokens: u64 = records.iter().map(|r| r.tokens_all.total).sum();
    md.push_str(&format!("\n{} runs, {:.1}M tokens, ${total:.2} at API prices.\n", records.len(), tokens as f64 / 1e6));
    md
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends tokens on the person's own engine login: run by hand (scripts/quality-gate.sh)"]
async fn behaviour_eval() {
    let threshold: f64 = env("EQUIPE_EVAL_THRESHOLD").and_then(|t| t.parse().ok()).unwrap_or(10.0);
    let out = env("EQUIPE_EVAL_OUT").map(PathBuf::from).unwrap_or_else(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/quality-gate").join(now.to_string())
    });
    std::fs::create_dir_all(&out).unwrap();
    if let Some(kept) = env("EQUIPE_EVAL_REPORT") {
        let records: Vec<Record> = kept.split(',').flat_map(|p| std::fs::read_to_string(p.trim()).unwrap_or_else(|e| panic!("{p}: {e}")).lines().filter_map(|l| serde_json::from_str(l).ok()).collect::<Vec<_>>()).collect();
        let md = report(&records, threshold, &format!("From {kept}.\n"));
        std::fs::write(out.join("report.md"), &md).unwrap();
        println!("{md}\nreport: {}", out.join("report.md").display());
        return;
    }
    let mut variants = Vec::new();
    if let Some(a) = env("EQUIPE_EVAL_A") {
        variants.push(Variant { name: "A".into(), binary: PathBuf::from(a) });
    }
    variants.push(Variant { name: "B".into(), binary: env("EQUIPE_EVAL_B").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_equipe"))) });
    for v in &variants {
        assert!(v.binary.exists(), "variant {}: no binary at {}", v.name, v.binary.display());
    }
    let wanted: Option<Vec<String>> = env("EQUIPE_EVAL_TASKS").map(|t| t.split(',').map(|s| s.trim().to_owned()).collect());
    let tasks: Vec<Task> = TASKS.iter().copied().filter(|t| wanted.as_ref().is_none_or(|w| w.iter().any(|n| n == t.name))).collect();
    assert!(!tasks.is_empty(), "no task named in EQUIPE_EVAL_TASKS; there are: {}", TASKS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "));
    let runs: u32 = env("EQUIPE_EVAL_RUNS").and_then(|n| n.parse().ok()).unwrap_or(3);
    let jobs: usize = env("EQUIPE_EVAL_JOBS").and_then(|n| n.parse().ok()).unwrap_or(1).max(1);
    let engine = match env("EQUIPE_EVAL_ENGINE").as_deref() {
        None | Some("claude") => Some(Engine::Claude),
        Some("codex") => Some(Engine::Codex),
        Some("default") => None,
        Some(other) => panic!("EQUIPE_EVAL_ENGINE: claude, codex or default, not {other}"),
    };
    if engine == Some(Engine::Claude) {
        let status = equipe_providers::claude_code::auth_status(equipe_providers::claude_code::DEFAULT_BINARY).await.ok().flatten();
        assert!(status.is_some_and(|s| s.logged_in), "Claude Code isn't signed in: sign in with `claude` as you normally do");
    }
    let setup = Setup { engine, model: env("EQUIPE_EVAL_MODEL"), keep: env("EQUIPE_EVAL_KEEP").is_some() };
    let about = format!(
        "Variants: {}. Tasks: {}. {runs} run(s) each, engine {}, models {}.\n",
        variants.iter().map(|v| format!("{} = `{}`", v.name, v.binary.display())).collect::<Vec<_>>().join(", "),
        tasks.len(),
        engine.map(|e| format!("{e:?}")).unwrap_or_else(|| "each role's own".into()),
        setup.model.clone().unwrap_or_else(|| "each role's own".into())
    );
    println!("{about}report folder: {}", out.display());
    let jsonl = out.join("runs.jsonl");
    let gate = Arc::new(tokio::sync::Semaphore::new(jobs));
    let mut handles = Vec::new();
    // A and B side by side for each attempt, so a slow hour weighs on both.
    // A run that panics is a failed run, never the end of the others.
    for task in &tasks {
        for attempt in 1..=runs {
            for variant in &variants {
                let (task, variant, setup, out, gate) = (*task, variant.clone(), setup.clone(), out.clone(), gate.clone());
                let name = variant.name.clone();
                handles.push((task, name, attempt, tokio::spawn(async move {
                    let _turn = gate.acquire_owned().await.unwrap();
                    run_once(task, variant, attempt, setup, out).await
                })));
            }
        }
    }
    let mut records = Vec::new();
    for (task, variant, attempt, h) in handles {
        let rec = match h.await {
            Ok(rec) => rec,
            Err(e) => broken(&task, &variant, attempt, &format!("the run broke off: {e}")),
        };
        println!(
            "{} {}{}: {} · {}k tokens (member {}k) · ${:.2} · {:.1} min{}",
            rec.task,
            rec.variant,
            rec.attempt,
            if rec.pass { "pass" } else { "FAIL" },
            rec.tokens_all.total / 1000,
            rec.tokens.total / 1000,
            rec.cost_usd,
            rec.wall_secs / 60.0,
            rec.checks.iter().filter(|c| !c.ok).map(|c| format!(" · ✗ {}", c.name)).collect::<String>()
        );
        let mut line = serde_json::to_string(&rec).unwrap();
        line.push('\n');
        use std::io::Write as _;
        std::fs::OpenOptions::new().create(true).append(true).open(&jsonl).unwrap().write_all(line.as_bytes()).unwrap();
        records.push(rec);
    }
    let md = report(&records, threshold, &about);
    std::fs::write(out.join("report.md"), &md).unwrap();
    println!("\n{md}\nreport: {}", out.join("report.md").display());
    if variants.len() == 2 {
        let flags = verdict(&records, threshold);
        assert!(flags.is_empty(), "the quality gate flagged B: {flags:?} — see {}", out.join("report.md").display());
    }
}

// ── What runs in every `cargo test`: the fixtures, graders and verdict, with no engine ──

fn have(program: &str) -> bool {
    std::process::Command::new(program).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("equipe-qg-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn task(name: &str) -> Task {
    *TASKS.iter().find(|t| t.name == name).unwrap()
}

#[test]
fn there_are_eleven_tasks_across_the_roles_with_names_of_their_own() {
    assert_eq!(TASKS.len(), 11);
    let mut names: Vec<&str> = TASKS.iter().map(|t| t.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), TASKS.len());
    for role in ["pm", "designer", "frontend", "backend", "qa", "data", "security", "marketing"] {
        assert!(TASKS.iter().any(|t| t.role == role), "{role}");
        assert!(equipe_core::roles::role(role).is_some(), "{role} is a role");
    }
}

#[test]
fn the_fixtures_have_the_problems_their_tasks_plant() {
    let root = scratch("fixtures");
    if have("python3") {
        let fx = tasks::setup(&task("backend-fix-failing-test"), &root);
        let repo = fx.repo.unwrap();
        assert!(!harness::run_in(&repo, "python3", &["-m", "unittest"], &[]).0, "the test fails as it starts");
        std::fs::write(repo.join("stats.py"), tasks::STATS_PY.replace("sum(values) // len", "sum(values) / len")).unwrap();
        assert!(harness::run_in(&repo, "python3", &["-m", "unittest"], &[]).0, "and passes once the code is fixed");

        let fx = tasks::setup(&task("backend-wait-for-server"), &root);
        let repo = fx.repo.unwrap();
        assert!(!harness::run_in(&repo, "python3", &["-m", "unittest"], &[]).0);
        std::fs::write(repo.join("server.py"), tasks::SERVER_PY.replace("\"okay\"", "\"ok\"")).unwrap();
        assert!(harness::run_in(&repo, "python3", &["-m", "unittest"], &[]).0);
        assert!(std::fs::read_to_string(repo.join(".port")).unwrap().trim().parse::<u16>().is_ok());

        let fx = tasks::setup(&task("qa-planted-bug"), &root);
        let found = scratch("found");
        std::fs::write(found.join("test_found.py"), "import unittest\nfrom pricing import discount\n\nclass Found(unittest.TestCase):\n    def test_fifty_gets_save10(self):\n        self.assertEqual(discount(50, 'SAVE10'), 5.0)\n").unwrap();
        let bug = fx.repo.unwrap().display().to_string();
        let fixed = fx.fixed.unwrap().display().to_string();
        assert!(!harness::run_in(&found, "python3", &["-m", "unittest", "test_found"], &[("PYTHONPATH", &bug)]).0, "the boundary bug is there");
        assert!(harness::run_in(&found, "python3", &["-m", "unittest", "test_found"], &[("PYTHONPATH", &fixed)]).0, "and not in the fixed copy");
    }
    if have("node") {
        let fx = tasks::setup(&task("frontend-clear-button"), &root);
        assert!(harness::run_in(&fx.repo.unwrap(), "node", &["--test"], &[]).0, "its tests pass as it starts");
    }
    let (_, weeks) = tasks::signups();
    assert_eq!(weeks.iter().map(|(w, _)| *w).collect::<Vec<_>>(), [40, 41, 42, 43, 44], "October 2026's ISO weeks");
    let top = weeks.iter().map(|(_, c)| *c).max().unwrap();
    assert_eq!(weeks.iter().filter(|(_, c)| *c == top).map(|(w, _)| *w).collect::<Vec<_>>(), [42], "one busiest week");
    let fx = tasks::setup(&task("data-weekly-signups"), &root);
    assert_eq!(fx.expected[0], "42");
    let csv = std::fs::read_to_string(fx.repo.unwrap().join("signups.csv")).unwrap();
    assert_eq!(csv.lines().count() as u32 - 1, weeks.iter().map(|(_, c)| c).sum::<u32>());
    let fx = tasks::setup(&task("security-review"), &root);
    assert!(std::fs::read_to_string(fx.repo.unwrap().join("config.py")).unwrap().contains(tasks::API_KEY));
    let _ = std::fs::remove_dir_all(&root);
}

/// An outcome as a run that handed `ticket` in leaves it.
fn outcome_with(root: &Path, ticket: equipe_core::protocol::Ticket) -> Outcome {
    let mut state = State::default();
    state.tickets.insert(ticket.id, ticket);
    Outcome { state, histories: BTreeMap::new(), transcripts: Vec::new(), wall: std::time::Duration::from_secs(60), root: root.to_owned(), home_dir: root.join("home") }
}

#[test]
fn the_grader_passes_the_work_done_and_fails_the_work_not_done() {
    if !have("python3") {
        return;
    }
    use equipe_core::protocol::{Ticket, TicketStatus};
    let t = task("backend-fix-failing-test");
    let root = scratch("grade");
    let fx = tasks::setup(&t, &root);
    let repo = fx.repo.clone().unwrap();
    let worktree = root.join("wt");
    harness::git(&repo, &["worktree", "add", "-q", "-b", "equipe/backend-t1-x", &worktree.to_string_lossy()]);
    let ticket: Ticket = serde_json::from_value(json!({
        "id": 1, "title": "Fix", "body": "", "status": "review", "kind": "code", "owner": null, "assignee": "backend",
        "project": repo, "depends_on": [], "branch": "equipe/backend-t1-x", "worktree": worktree, "created_by": "you", "note": null,
    }))
    .unwrap();
    let failed = tasks::grade(&t, &fx, &outcome_with(&root, ticket.clone()));
    assert!(failed.iter().any(|c| c.name == "tests pass" && !c.ok), "nothing fixed yet: {failed:?}");
    std::fs::write(worktree.join("stats.py"), tasks::STATS_PY.replace("sum(values) // len", "sum(values) / len")).unwrap();
    let passed = tasks::grade(&t, &fx, &outcome_with(&root, ticket.clone()));
    assert!(passed.iter().all(|c| c.ok || !c.required), "{passed:?}");
    std::fs::write(worktree.join("test_stats.py"), "").unwrap();
    let cheated = tasks::grade(&t, &fx, &outcome_with(&root, ticket.clone()));
    assert!(cheated.iter().any(|c| c.name == "the test is unchanged" && !c.ok), "emptying the test is caught: {cheated:?}");
    let todo = Ticket { status: TicketStatus::InProgress, ..ticket };
    assert!(tasks::grade(&t, &fx, &outcome_with(&root, todo)).iter().any(|c| c.name == "handed in" && !c.ok));
    let _ = std::fs::remove_dir_all(&root);
}

fn rec(task: &str, variant: &str, pass: bool, kept: Option<bool>) -> Record {
    Record {
        task: task.into(),
        role: "backend".into(),
        variant: variant.into(),
        attempt: 1,
        pass,
        checks: vec![],
        compliance: [("estimate on ticket work".to_owned(), kept)].into(),
        skills: vec![],
        tokens: TokenRecord::default(),
        tokens_all: TokenRecord { total: 1000, ..Default::default() },
        cost_usd: 0.1,
        wall_secs: 60.0,
        tool_calls: 3,
        transcripts: 1,
        errors: vec![],
    }
}

#[test]
fn the_verdict_flags_a_task_passed_less_often_and_a_rule_kept_less_often_beyond_the_threshold() {
    let same = [rec("t", "A", true, Some(true)), rec("t", "B", true, Some(true))];
    assert!(verdict(&same, 10.0).is_empty());
    let worse = [rec("t", "A", true, Some(true)), rec("t", "A", true, Some(true)), rec("t", "B", true, Some(true)), rec("t", "B", false, Some(true))];
    assert_eq!(verdict(&worse, 10.0), [Flag::Task { task: "t".into(), a: 1.0, b: 0.5 }]);
    let better = [rec("t", "A", false, Some(true)), rec("t", "B", true, Some(true))];
    assert!(verdict(&better, 10.0).is_empty(), "better is never flagged");
    let mut rule = vec![rec("t", "A", true, Some(true)), rec("t", "B", true, Some(false))];
    assert_eq!(verdict(&rule, 10.0), [Flag::Metric { metric: "estimate on ticket work".into(), a: 1.0, b: 0.0 }]);
    rule = (0..10).map(|_| rec("t", "A", true, Some(true))).chain((0..10).map(|i| rec("t", "B", true, Some(i != 0)))).collect();
    assert!(verdict(&rule, 10.0).is_empty(), "a 10-point drop is within a 10-point threshold");
    assert_eq!(verdict(&rule, 5.0).len(), 1);
    let not_in_play = [rec("t", "A", true, Some(true)), rec("t", "B", true, None)];
    assert!(verdict(&not_in_play, 10.0).is_empty(), "a rule not in play on B is no drop");
    let md = report(&worse, 10.0, "test\n");
    assert!(md.contains("**Verdict: FAIL**") && md.contains("task `t`: passed 100% on A, 50% on B"), "{md}");
    assert!(report(&[rec("t", "B", true, None)], 10.0, "").contains("no verdict"));
    let line = serde_json::to_string(&worse[3]).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&line).unwrap(), worse[3], "a kept run reads back");
}

#[test]
fn without_a_transcript_a_members_steps_come_from_the_daemons_events() {
    let events = vec![
        (1.0, Event::ToolUse { session: 7, tool: "Bash".into(), target: Some("sleep 30 && ls".into()), label: None, background: false }),
        (2.0, Event::ToolUse { session: 8, tool: "Bash".into(), target: Some("sleep 30".into()), label: None, background: false }),
        (3.0, Event::ToolUse { session: 7, tool: "mcp__equipe__estimate".into(), target: None, label: None, background: false }),
        (4.0, Event::ToolUse { session: 7, tool: "Bash".into(), target: Some("sleep 60".into()), label: None, background: true }),
    ];
    let r = quality::rules(&steps_from_events(&events, 7));
    assert_eq!((r.tool_calls, r.foreground_sleeps, r.estimates), (3, 1, 1), "only its own, and a wait in the background is fine");
}

#[test]
fn ticket_work_is_timed_from_its_start_to_its_end() {
    let start = |at: f64| (at, Event::WorkStarted { session: 7, ticket: 1, title: "x".into(), project: PathBuf::from("/p"), workspace: None });
    let end = |at: f64| (at, Event::WorkEnded { session: 7, ticket: 1 });
    let notice = |at: f64| (at, Event::Notice { session: 8, text: "x".into() });
    assert_eq!(work_seconds(&[start(10.0), end(70.0), start(100.0), end(130.0)], 7), 90.0);
    assert_eq!(work_seconds(&[start(10.0), notice(250.0)], 7), 240.0, "still at it when the run ended");
    assert_eq!(work_seconds(&[start(10.0), end(70.0)], 8), 0.0);
}
