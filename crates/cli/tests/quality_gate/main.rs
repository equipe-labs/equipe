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
//! - `EQUIPE_EVAL_REVIEW=off`: no reviewer on the design tasks (`design_grade.rs`)
//! - `EQUIPE_EVAL_REVIEW_MODEL`: the reviewer's model (default `opus`)
//! - `EQUIPE_EVAL_CHROME`: the Chrome the design tasks' pages are measured in
//! - `EQUIPE_EVAL_REPORT=<runs.jsonl>[,<more.jsonl>]`: only write the report again from runs kept

#![cfg(unix)]

mod design_grade;
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
    /// The reviewer's judgement, on a task that has one (`design_grade`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewRecord {
    pub score: f64,
    pub max: f64,
    pub scores: serde_json::Value,
    /// The reviewer's own tokens and cost: not in the run's.
    pub tokens: u64,
    pub cost_usd: f64,
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
    "no block on the person",
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
    if transcripts.is_empty()
        && let Some(session) = out.state.session(task.role)
    {
        rules = quality::rules(&steps_from_events(&out.state.events, session));
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
    // No task needs the person beyond its ticket: each put-down is counted.
    let blocked = out.state.person.blocks.iter().filter(|(id, _)| out.state.tickets_of(task.role).any(|t| t.id == *id)).count();
    put("no block on the person", worked_ticket.then_some(blocked == 0));
    (m, skills, rules)
}

fn record(task: &Task, variant: &Variant, attempt: u32, checks: &[Check], out: &Outcome, review: Option<&design_grade::Review>) -> Record {
    let (compliance, skills, rules) = compliance(task, out);
    let mut mine = Tokens::default();
    let mut all = Tokens::default();
    for t in &out.transcripts {
        all.add(t.tokens);
        if t.role.as_deref() == Some(task.role) {
            mine.add(t.tokens);
        }
    }
    // Sessions with no transcript (Codex): the turns' own usage, as the
    // daemon told it — beside the transcripts of any Claude Code run of the
    // same squad (a critic on the other engine, say).
    let codex: Vec<SessionId> = out.state.events.iter().filter_map(|(_, e)| match e {
        Event::SessionStarted { session, engine: Engine::Codex, .. } => Some(*session),
        _ => None,
    }).collect();
    {
        for (session, usage) in out.state.usage.iter().filter(|(s, _)| out.transcripts.is_empty() || codex.contains(s)) {
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
        review: review.map(|r| ReviewRecord { score: r.score, max: r.max, scores: r.scores.clone(), tokens: r.tokens, cost_usd: r.cost_usd }),
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
    let mut checks = tasks::grade(&task, &fx, &outcome);
    // The reviewer's judgement, where a task has one: it informs, never fails a run.
    let review = if env("EQUIPE_EVAL_REVIEW").as_deref() == Some("off") { None } else { tasks::review(&task, &outcome, &dir.join("review")) };
    if let Some(r) = &review {
        checks.push(design_grade::review_check(r));
    }
    let rec = record(&task, &variant, attempt, &checks, &outcome, review.as_ref().and_then(|r| r.as_ref().ok()));
    // What a design task's page measured, and its pictures, kept beside the record.
    if let Ok(files) = std::fs::read_dir(root.join("measured")) {
        let kept = dir.join("measured");
        let _ = std::fs::create_dir_all(&kept);
        for f in files.flatten() {
            let _ = std::fs::copy(f.path(), kept.join(f.file_name()));
        }
    }
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
        review: None,
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
    let reviewed: Vec<&Record> = records.iter().filter(|r| r.review.is_some()).collect();
    if !reviewed.is_empty() {
        md.push_str("\n## The reviewer's scores\n\nA reviewer with no context but a fixed rubric (`design_grade.rs`) and the pictures; it informs and never fails a run. Mean share of the most a run could score, and each run's.\n\n| task |");
        for v in &variants {
            md.push_str(&format!(" {v} |"));
        }
        md.push_str("\n|---|");
        for _ in &variants {
            md.push_str("---|");
        }
        md.push('\n');
        let mut tasks: Vec<&str> = reviewed.iter().map(|r| r.task.as_str()).collect();
        tasks.sort();
        tasks.dedup();
        for task in tasks {
            md.push_str(&format!("| `{task}` |"));
            for v in &variants {
                let runs: Vec<&ReviewRecord> = reviewed.iter().filter(|r| r.task == task && r.variant == *v).filter_map(|r| r.review.as_ref()).collect();
                let each: Vec<String> = runs.iter().map(|r| format!("{}/{}", r.score, r.max)).collect();
                let share = mean(runs.iter().map(|r| r.score / r.max));
                md.push_str(&format!(" {} ({}) |", pct(share), each.join(", ")));
            }
            md.push('\n');
        }
    }
    let total: f64 = records.iter().map(|r| r.cost_usd).sum();
    let tokens: u64 = records.iter().map(|r| r.tokens_all.total).sum();
    md.push_str(&format!("\n{} runs, {:.1}M tokens, ${total:.2} at API prices.\n", records.len(), tokens as f64 / 1e6));
    if !reviewed.is_empty() {
        let (t, c): (u64, f64) = reviewed.iter().filter_map(|r| r.review.as_ref()).fold((0, 0.0), |(t, c), r| (t + r.tokens, c + r.cost_usd));
        md.push_str(&format!("The reviewer besides: {:.1}M tokens, ${c:.2}.\n", t as f64 / 1e6));
    }
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
fn there_are_thirteen_tasks_across_the_roles_with_names_of_their_own() {
    assert_eq!(TASKS.len(), 13);
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
        review: None,
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
    let notice = |at: f64| (at, Event::Notice { undo: None, session: 8, text: "x".into() });
    assert_eq!(work_seconds(&[start(10.0), end(70.0), start(100.0), end(130.0)], 7), 90.0);
    assert_eq!(work_seconds(&[start(10.0), notice(250.0)], 7), 240.0, "still at it when the run ended");
    assert_eq!(work_seconds(&[start(10.0), end(70.0)], 8), 0.0);
}

#[test]
fn made_up_numbers_quotes_and_proof_are_caught_and_the_briefs_own_facts_are_not() {
    let brief = format!("{}\n{}", tasks::QUILLO_README, tasks::QUILLO_TICKET);
    let honest = "Quillo. Upload a recording up to 2 hours long. Free: 3 meetings a month. Team: $8 per person per month. $0 to start. The beta opens 3 March. Step 1, 2, 3. 9:41 Weekly sync · 3 decisions · 5 to-dos. © 2026 Quillo";
    assert_eq!(design_grade::made_up(honest, "<p>", &brief), Vec::<String>::new(), "the brief's numbers, a year, a sample's");
    for (text, what) in [
        ("Trusted by teams everywhere", "trusted by"),
        ("Loved by product teams", "loved by"),
        ("Rated 4.9/5 by early users", "rated"),
        ("★★★★★", "★"),
        ("Save 5 hours a week", "a count"),
        ("Join 10,000+ teams", "a count with +"),
        ("Notes 3x faster", "a multiple"),
        ("98% accurate", "a percentage"),
        ("Used by 1,200 companies", "used by"),
        ("Only $12 a month", "a price"),
        ("Summaries in 30 seconds", "a count"),
    ] {
        let found = design_grade::made_up(text, "<p>", &brief);
        assert!(found.iter().any(|f| f.contains(what)), "{text:?} → {found:?}");
    }
    assert!(!design_grade::made_up("x", "<blockquote>“Great”</blockquote>", &brief).is_empty(), "a quote is a testimonial on a page nobody has used");
    assert!(design_grade::made_up("It generated a summary", "", &brief).is_empty(), "whole words only: generated isn't rated");
}

#[test]
fn the_report_shows_the_reviewers_scores_and_they_never_fail_a_run() {
    let reviewed = |variant: &str, score: f64| Record { review: Some(ReviewRecord { score, max: 30.0, scores: json!({}), tokens: 40_000, cost_usd: 0.2 }), ..rec("t", variant, true, None) };
    let records = [reviewed("A", 15.0), reviewed("B", 24.0), reviewed("B", 18.0)];
    let md = report(&records, 10.0, "");
    assert!(md.contains("## The reviewer's scores") && md.contains("| `t` | 50% (15/30) | 70% (24/30, 18/30) |"), "{md}");
    assert!(md.contains("The reviewer besides: 0.1M tokens, $0.60."), "{md}");
    assert!(design_grade::review_check(&Err("no pictures".into())).required == false, "a review informs, never fails a run");
    let line = serde_json::to_string(&records[0]).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&line).unwrap(), records[0]);
    let older = line.replace(&format!(",\"review\":{}", serde_json::to_string(records[0].review.as_ref().unwrap()).unwrap()), "");
    assert!(serde_json::from_str::<Record>(&older).unwrap().review.is_none(), "runs kept before reviews read back");
}

#[test]
fn on_codex_the_reviewer_gets_the_pictures_attached_and_its_tokens_from_its_events() {
    for rubric in [design_grade::LANDING_RUBRIC, design_grade::EMPTY_STATE_RUBRIC] {
        let codex = design_grade::for_codex(rubric);
        assert!(!codex.contains("Read tool") && codex.contains("The images are attached") && codex.contains("only one JSON object"), "the same rubric, read from attachments");
        assert_eq!(codex.replace("The images are attached, in the order listed.", ""), rubric.replace("Open every image with the Read tool first.", ""), "and nothing else changed");
    }
    let events = "{\"type\":\"thread.started\"}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1200,\"cached_input_tokens\":800,\"output_tokens\":150}}\nnot json\n";
    assert_eq!(design_grade::codex_tokens(events), 1350, "input (its cached part included) and output");
}

#[test]
fn a_codex_members_tokens_count_beside_a_claude_critics_transcript() {
    use equipe_core::protocol::{Ticket, Usage};
    let ticket: Ticket = serde_json::from_value(json!({
        "id": 1, "title": "x", "body": "", "status": "review", "kind": "task", "owner": null, "assignee": "designer",
        "project": null, "depends_on": [], "branch": null, "worktree": null, "created_by": "you", "note": null,
    }))
    .unwrap();
    let mut out = outcome_with(&scratch("codex-tokens"), ticket);
    out.state.sessions.insert("designer".into(), 7);
    let started: Event = serde_json::from_value(json!({ "type": "session_started", "session": 7, "engine": "codex", "project": "/p", "model": null, "workspace": null })).unwrap();
    assert!(matches!(started, Event::SessionStarted { engine: Engine::Codex, .. }), "a Codex session's start: {started:?}");
    out.state.events.push((0.0, started));
    out.state.usage.insert(7, vec![Usage { input_tokens: Some(400), cache_tokens: Some(1000), output_tokens: Some(100), ..Default::default() }]);
    let critic = quality::Transcript { tokens: Tokens { input: 10, cache_write: 0, cache_read: 20, output: 5 }, ..Default::default() };
    out.transcripts.push(critic);
    let rec = record(&task("designer-empty-state"), &Variant { name: "B".into(), binary: PathBuf::new() }, 1, &[], &out, None);
    assert_eq!(rec.tokens_all.total, 1500 + 35, "the member on Codex and the critic on Claude");
    assert_eq!(rec.tokens.total, 1500);
}

#[test]
fn without_a_transcript_a_skill_read_among_the_first_steps_counts_as_opened_first() {
    use equipe_core::protocol::Ticket;
    let ticket: Ticket = serde_json::from_value(json!({
        "id": 1, "title": "x", "body": "", "status": "review", "kind": "task", "owner": null, "assignee": "designer",
        "project": null, "depends_on": [], "branch": null, "worktree": null, "created_by": "you", "note": null,
    }))
    .unwrap();
    let mut out = outcome_with(&scratch("early"), ticket);
    out.state.sessions.insert("designer".into(), 7);
    let used = |target: &str| (1.0, Event::ToolUse { session: 7, tool: "exec_command".into(), target: Some(target.into()), label: None, background: false });
    out.state.events = vec![used("ls"), used("cat /h/skills/0.1/designer-x/skills/design-process/SKILL.md")];
    assert!(tasks::opened_early(&out, "designer", "design-process").is_some());
    out.state.events = (0..tasks::EARLY).map(|_| used("ls")).chain([used("cat /h/skills/design-process/SKILL.md")]).collect();
    assert!(tasks::opened_early(&out, "designer", "design-process").is_none(), "read too late");
    // On Codex the events cut commands short, so it informs and never fails a run.
    assert!(!tasks::on_codex(&out, "designer"));
    let started: Event = serde_json::from_value(json!({ "type": "session_started", "session": 7, "engine": "codex", "project": "/p", "model": null, "workspace": null })).unwrap();
    out.state.events.insert(0, (0.0, started));
    assert!(tasks::on_codex(&out, "designer"));
    let root = scratch("codex-grade");
    let fx = tasks::setup(&task("designer-empty-state"), &root);
    let checks = tasks::grade(&task("designer-empty-state"), &fx, &out);
    let opened = checks.iter().find(|c| c.name == "design-process opened first").unwrap();
    assert!(!opened.ok && !opened.required, "{opened:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_reviewers_scores_are_read_from_its_reply() {
    let said = "Here you go:\n{\"hierarchy\": 4, \"typography\": 3, \"layout\": 4, \"specificity\": 2, \"phone\": 5, \"finish\": 3, \"worst\": \"a {brace} in words\"}\nthanks";
    let v = design_grade::first_object(said).unwrap();
    assert_eq!(design_grade::total(&v), (21.0, 30.0));
    assert_eq!(design_grade::total(&json!({"a": 9, "b": 0, "c": 2})), (2.0, 5.0), "only 1–5 scores count");
    assert!(design_grade::first_object("no json").is_none());
    for rubric in [design_grade::LANDING_RUBRIC, design_grade::EMPTY_STATE_RUBRIC] {
        assert!(rubric.contains("Read tool") && rubric.contains("only one JSON object"), "each rubric asks for the pictures read and one JSON answer");
    }
}

#[test]
fn the_landing_page_grader_passes_a_sound_page_and_names_what_is_wrong_with_a_bad_one() {
    if !design_grade::can_measure() {
        return;
    }
    let brief = format!("{}\n{}", tasks::QUILLO_README, tasks::QUILLO_TICKET);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/quality_gate/fixtures");
    let grade = |name: &str| {
        let page = fixtures.join(name);
        let out = scratch("landing");
        let m = design_grade::measure(&page, &out).unwrap();
        assert!(out.join("1440-1.png").is_file() && out.join("360-1.png").is_file(), "pictures for the reviewer");
        let checks = design_grade::landing_checks(&m, &brief, &std::fs::read_to_string(&page).unwrap());
        let _ = std::fs::remove_dir_all(&out);
        checks
    };
    let good = grade("landing-good.html");
    assert!(good.iter().all(|c| c.ok), "{good:?}");
    let bad = grade("landing-bad.html");
    for name in ["fits at 360, 768 and 1440", "text contrast meets WCAG AA", "focus shows on every control", "sound structure", "the sections asked for", "only the brief's facts"] {
        assert!(bad.iter().any(|c| c.name == name && !c.ok && c.required), "{name}: {bad:?}");
    }
    let structure = bad.iter().find(|c| c.name == "sound structure").unwrap();
    for wrong in ["no lang", "viewport", "no title", "0 main", "0 h1", "images without alt", "fields without a name", "without a name"] {
        assert!(structure.detail.contains(wrong), "{wrong}: {}", structure.detail);
    }
}

#[test]
fn the_reviewer_sees_the_tickets_own_mockups_not_check_renders_or_another_tickets() {
    let root = scratch("mockups");
    let worktree = root.join("wt");
    let other = root.join("home/squads/eval/home/outputs/T-2 Top bar");
    for p in [worktree.join("outputs/T-1 Empty/design/checks/saved/360.png"), worktree.join("outputs/T-1 Empty/options/a.png"), worktree.join("outputs/T-1 Empty/mockup/empty.png"), other.join("after.png")] {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
    }
    let ticket: equipe_core::protocol::Ticket = serde_json::from_value(json!({
        "id": 1, "title": "Empty", "body": "", "status": "waiting", "kind": "task", "owner": null, "assignee": "designer",
        "project": null, "depends_on": [], "branch": null, "worktree": worktree, "created_by": "you", "note": null,
    }))
    .unwrap();
    let out = outcome_with(&root, ticket.clone());
    let png = |p: &Path| p.extension().is_some_and(|x| x == "png");
    let seen = tasks::mockups(&out, Some(&ticket), png);
    assert_eq!(seen, [worktree.join("outputs/T-1 Empty/mockup/empty.png"), worktree.join("outputs/T-1 Empty/options/a.png")], "mockups first; no check renders, nothing of T-2");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_automatic_person_answers_a_ticket_put_down_for_them_twice_then_leaves_it_waiting() {
    use equipe_core::protocol::{Ticket, TicketStatus};
    let waiting = |note: &str| -> Ticket {
        serde_json::from_value(json!({
            "id": 1, "title": "Empty", "body": "", "status": "waiting", "kind": "task", "owner": null, "assignee": "designer",
            "project": null, "depends_on": [], "branch": null, "worktree": null, "created_by": "you", "note": note,
        }))
        .unwrap()
    };
    let mut state = State::default();
    let first = waiting("Waiting on you:\n- [ ] the save control's name");
    assert_eq!(state.person.on_ticket(&first), Some(harness::NOTHING_MORE));
    assert_eq!(state.person.on_ticket(&first), None, "the same put-down is answered once");
    state.tickets.insert(1, first.clone());
    assert!(!state.settled("designer"), "answered: the member goes on");
    assert!(state.person.on_ticket(&waiting("Waiting on you:\n- [ ] say \"not designed yet\"")).is_some());
    assert_eq!(state.person.on_ticket(&waiting("Waiting on you:\n- [ ] a third thing")), None, "after two answers it waits, as a person with nothing more would");
    assert!(state.settled("designer"), "and the run can end on it");
    assert_eq!(state.person.blocks.len(), 2);
    let working = Ticket { status: TicketStatus::InProgress, ..waiting("x") };
    assert_eq!(State::default().person.on_ticket(&working), None);
    // The rule metric counts each put-down of the member's own tickets.
    let t = task("designer-empty-state");
    let mut out = outcome_with(&scratch("blocks"), Ticket { status: TicketStatus::Review, ..first });
    out.state.person.blocks.push((1, "x".into()));
    assert_eq!(compliance(&t, &out).0.get("no block on the person"), Some(&Some(false)));
    out.state.person.blocks.clear();
    assert_eq!(compliance(&t, &out).0.get("no block on the person"), Some(&Some(true)));
}

#[test]
fn the_landing_page_task_starts_from_its_brief_alone() {
    let root = scratch("landing-fixture");
    let fx = tasks::setup(&task("designer-landing-page"), &root);
    let repo = fx.repo.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), tasks::QUILLO_README);
    assert!(!repo.join("index.html").exists(), "the page is the member's to make");
    assert!(fx.expected[0].contains("Join the beta") && fx.expected[0].contains("$8"), "the grader knows the brief and the ticket");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn members_find_their_own_variants_equipe_first_on_path() {
    let dir = scratch("path");
    let binary = dir.join("equipe-A");
    std::fs::write(&binary, "").unwrap();
    let path = harness::variant_path(&binary, &dir.join("bin"), Some(std::ffi::OsStr::new("/usr/bin:/bin")));
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    assert_eq!(dirs, [dir.join("bin"), "/usr/bin".into(), "/bin".into()]);
    assert_eq!(std::fs::read_link(dir.join("bin/equipe")).unwrap(), binary.canonicalize().unwrap());
    harness::variant_path(&binary, &dir.join("bin"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bytecode_python_leaves_is_not_counted_as_changed() {
    // Running the tests writes `__pycache__/` beside the code wherever Python
    // isn't told to keep it elsewhere (CI's runners do): the grader's trace,
    // not the member's work.
    let dir = scratch("pyc");
    let base = harness::repo(&dir, &[("stats.py", "x = 1\n")]);
    std::fs::write(dir.join("stats.py"), "x = 2\n").unwrap();
    std::fs::create_dir_all(dir.join("__pycache__")).unwrap();
    std::fs::write(dir.join("__pycache__/stats.cpython-312.pyc"), "bytecode").unwrap();
    std::fs::write(dir.join("loose.pyc"), "bytecode").unwrap();
    assert_eq!(harness::changed_files(&dir, &base), ["stats.py"]);
}

#[test]
fn the_plan_use_run_starts_on_a_week_whose_heaviest_use_is_the_persons_own_folder() {
    let root = scratch("plan-use");
    let t = task("pm-plan-use");
    let fx = tasks::setup(&t, &root);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let look = equipe_core::plan_use::look(&root.join("home"), Engine::Claude, equipe_core::protocol::plan_use::Span::Week, None, now, false);
    let outside = look.outside.as_ref().expect("the person's own use");
    let top = &outside.children[0];
    assert!(matches!(&top.who, equipe_core::protocol::plan_use::Who::Outside { folder: Some(f) } if f.ends_with(&fx.expected[0])), "{top:?}");
    assert!(outside.share_bp > 5_000, "the most of it: {}", outside.share_bp);
    assert_eq!(look.tickets.len(), 1, "and a ticket of the squad's");
    assert_eq!(tasks::percents("It took 57.6% — then 12 % and T-3 at 9%."), [57.6, 12.0, 9.0]);
    // Graded as a run leaves it: the PM read it, named the folder, and gave
    // only numbers the tool gave — Codex's week as the daemon read it among them.
    use equipe_core::protocol::{PlanUsage, PlanWindow};
    let answer = |reply: &str, called: bool| {
        let mut state = State::default();
        state.sessions.insert("pm".into(), 1);
        state.turns.insert(1, vec![reply.to_owned()]);
        let week = PlanUsage { five_hour: None, seven_day: Some(PlanWindow { used_percent: 7, resets_at: now + 86_400 }) };
        state.events.push((1.0, Event::PlanUsage { usage: week, engine: Engine::Codex }));
        if called {
            state.events.push((2.0, Event::ToolUse { session: 1, tool: "mcp__equipe__plan_use".into(), target: None, label: None, background: false }));
        }
        let out = Outcome { state, histories: BTreeMap::new(), transcripts: Vec::new(), wall: std::time::Duration::from_secs(60), root: root.clone(), home_dir: root.join("home") };
        tasks::grade(&t, &fx, &out)
    };
    let share = |bp: u32| format!("{:.1}%", f64::from(bp) / 100.0);
    let good = format!("Mostly your own sessions outside equipe: /opt/kiwi-scraper took {}. Codex used 7% of its week.", share(top.share_bp));
    assert!(answer(&good, true).iter().all(|c| c.ok || !c.required), "{:?}", answer(&good, true));
    let made_up = format!("{good} About 42% went to reviews.");
    assert!(answer(&made_up, true).iter().any(|c| c.name == "no number it didn't read" && !c.ok));
    assert!(answer(&good, false).iter().any(|c| c.name == "called plan_use" && !c.ok));
    let _ = std::fs::remove_dir_all(&root);
}
