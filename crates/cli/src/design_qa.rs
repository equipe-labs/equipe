//! `equipe design precheck`, `qa-stats`, `qa-config` and `qa-replay`
//! (docs/DESIGN-QA.md): design QA's deterministic precheck run on a ticket's
//! worktree — the daemon runs it on each hand-in with changed screens — what
//! design QA costs and decides in a squad, its settings, and past design QA
//! tickets replayed through the new pipeline, read only. The judging is
//! equipe's core's (`design::precheck`, `design_qa::gate`); here are git,
//! Chrome, the preview and the files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Args;
use equipe_core::design::{flows, precheck, qa_stats};
use equipe_core::design_qa::{self, Decision, GateMode, TierPolicy};
use equipe_core::t;
use serde_json::{Value, json};

#[derive(Args)]
pub struct PrecheckArgs {
    /// The ticket's worktree (default: here)
    #[arg(long, default_value = ".")]
    pub worktree: PathBuf,
    /// What to compare with, a branch or commit: the map's pictures are read where it meets HEAD (default: the main branch)
    #[arg(long)]
    pub base: Option<String>,
    /// Where the report and its pictures go
    #[arg(long, default_value = "precheck")]
    pub out: PathBuf,
    /// A preview already serving this worktree's code, `http://localhost:<port>` (else the map's `precheck.preview` is started)
    #[arg(long)]
    pub url: Option<String>,
    /// The ticket checked, for the report
    #[arg(long)]
    pub ticket: Option<u64>,
    /// How long rendering may take in all, in seconds
    #[arg(long, default_value_t = 900)]
    pub budget_secs: u64,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct QaStatsArgs {
    /// The squad (default: the one this folder is in)
    #[arg(long)]
    pub squad: Option<String>,
    /// From when: 7d, 24h, 2w, or a day (2026-11-01)
    #[arg(long, default_value = "30d")]
    pub since: String,
    /// Every ticket and the sums, as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct QaConfigArgs {
    /// The squad (default: the one this folder is in)
    #[arg(long)]
    pub squad: Option<String>,
    /// The precheck's gate: off (no precheck), shadow (report to the designer, decisions only recorded), on (it passes and sends back by itself)
    #[arg(long)]
    pub gate: Option<String>,
    /// Design QA's model tiers: economy (standard, deep when it takes judgment), quality (deep always), or <cheap>/<top>
    #[arg(long)]
    pub tiers: Option<String>,
}

#[derive(Args)]
pub struct QaReplayArgs {
    /// The squad (default: the one this folder is in)
    #[arg(long)]
    pub squad: Option<String>,
    /// Only these design QA tickets: 2077,2065
    #[arg(long, value_delimiter = ',')]
    pub tickets: Vec<u64>,
    /// How many of the latest design QA tickets with a verdict
    #[arg(long, default_value_t = 20)]
    pub last: usize,
    /// From when: 7d, 24h, 2w, or a day (2026-11-01)
    #[arg(long, default_value = "30d")]
    pub since: String,
    /// The precheck and its gate only: no model runs, nothing spent
    #[arg(long)]
    pub dry_run: bool,
    /// The replayed designer's model, without --dry-run
    #[arg(long, default_value = "sonnet")]
    pub designer_model: String,
    /// The replayed designer's effort, without --dry-run
    #[arg(long, default_value = "medium")]
    pub designer_effort: String,
    /// Where the replay is kept (default: ~/.equipe/design-qa-replay/<when>)
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// How long each precheck may render, in seconds
    #[arg(long, default_value_t = 900)]
    pub budget_secs: u64,
    /// The replay as JSON
    #[arg(long)]
    pub json: bool,
}

// ── git ──

fn git_cmd(dir: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(dir).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
    command
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_cmd(dir).args(args).output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// A file as committed at `rev`, if it was.
fn git_file(dir: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let out = git_cmd(dir).args(["show", &format!("{rev}:{path}")]).stderr(Stdio::null()).output().ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// The files changed since `base`, and those not committed yet.
fn changed_files(root: &Path, base: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = base.and_then(|b| git(root, &["diff", "--name-only", b, "HEAD"]).ok()).unwrap_or_default().lines().map(str::to_owned).collect();
    for line in git(root, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default().lines() {
        if let Some(path) = line.get(3..) {
            out.push(path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"').to_owned());
        }
    }
    out.retain(|p| !p.is_empty());
    out.sort();
    out.dedup();
    out
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

// ── the preview ──

/// The app served from a worktree by the map's `precheck.preview`, on a
/// port of its own; stopped with all it started.
struct Preview {
    child: tokio::process::Child,
    origin: String,
}

impl Preview {
    async fn start(command: &str, root: &Path, ready: Option<&str>, out: &Path) -> Result<Preview, String> {
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
        let command = command.replace("{port}", &port.to_string());
        eprintln!("{}", t!("design.precheck.preview", command = command));
        let log = std::fs::File::create(out.join("preview.log")).map_err(|e| e.to_string())?;
        let mut sh = tokio::process::Command::new("/bin/sh");
        sh.arg("-c").arg(&command).current_dir(root).env("PORT", port.to_string()).stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log).kill_on_drop(true);
        #[cfg(unix)]
        sh.process_group(0);
        let mut child = sh.spawn().map_err(|e| format!("sh: {e}"))?;
        let preview_path = ready.unwrap_or("/");
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if answers(port, preview_path).await {
                return Ok(Preview { child, origin: format!("http://localhost:{port}") });
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("`{command}` stopped ({status}){}; see {}", precheck::log_head(&out.join("preview.log")), out.join("preview.log").display()));
            }
            if Instant::now() > deadline {
                let preview = Preview { child, origin: String::new() };
                preview.stop().await;
                return Err(format!("`{command}` didn't answer on port {port} within 3 minutes"));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn stop(mut self) {
        if let Some(pid) = self.child.id() {
            equipe_core::group::end(pid, equipe_core::group::Signal::Terminate);
            tokio::time::sleep(Duration::from_secs(1)).await;
            equipe_core::group::end(pid, equipe_core::group::Signal::Kill);
        }
        let _ = self.child.wait().await;
    }
}

/// A changed file's text in the worktree, when it is there and readable.
fn read_changed(root: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(root.join(file)).ok()
}

/// Whether something serves HTTP on `port` at `path` — the first answer
/// waited for, as a dev server builds the page then.
async fn answers(port: u16, path: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(("localhost", port))).await else { return false };
    let ask = format!("GET {path} HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n");
    if stream.write_all(ask.as_bytes()).await.is_err() {
        return false;
    }
    let mut head = [0u8; 5];
    matches!(tokio::time::timeout(Duration::from_secs(90), stream.read_exact(&mut head)).await, Ok(Ok(_))) && &head == b"HTTP/"
}

// ── the precheck ──

/// One precheck: the worktree, what to compare with, where it goes.
pub(crate) struct Run {
    pub worktree: PathBuf,
    pub base: Option<String>,
    pub out: PathBuf,
    pub url: Option<String>,
    pub ticket: Option<u64>,
    pub budget: Duration,
}

fn key_of(s: &flows::Shot) -> String {
    format!("{}/{}/{}", s.flow, s.screen, s.state)
}

/// The precheck of `r.worktree`: its report, kept in `r.out` with the
/// gate's headline (as it would decide: nothing acts here).
pub(crate) async fn run(r: &Run) -> Result<precheck::Report, String> {
    let started = Instant::now();
    let root = equipe_core::worktree::repo_root(&r.worktree)?;
    std::fs::create_dir_all(&r.out).map_err(|e| format!("{}: {e}", r.out.display()))?;
    let out = std::path::absolute(&r.out).unwrap_or_else(|_| r.out.clone());
    let head = git(&root, &["rev-parse", "HEAD"]).ok();
    let base_ref = match &r.base {
        Some(b) => b.clone(),
        None => equipe_core::worktree::main_branch(&root)?,
    };
    let mut errors = Vec::new();
    let base = match git(&root, &["merge-base", "HEAD", &base_ref]) {
        Ok(b) => Some(b),
        Err(_) => {
            errors.push(format!("no commit in common with {base_ref}: nothing to compare with"));
            None
        }
    };
    let changed = changed_files(&root, base.as_deref());
    let design = root.join(equipe_core::design::DIR);
    let flows_rel = format!("{}/{}", equipe_core::design::DIR, flows::DIR);
    let mut input = precheck::Input { ticket: r.ticket, head, base: base.clone(), changed_files: changed.len(), ..precheck::Input::default() };
    let map: Option<Value> = std::fs::read_to_string(design.join(flows::DIR).join(flows::FILE)).ok().and_then(|t| serde_json::from_str(&t).ok());
    let Some(map) = map else {
        errors.push(format!("no UI map in the worktree ({flows_rel}/{}): nothing to render", flows::FILE));
        input.errors = errors;
        (input.touched.unmapped, input.touched.quiet) = precheck::split_unmapped(changed.iter().filter(|f| precheck::ui_file(f)).cloned().collect(), |f| read_changed(&root, f));
        input.secs = started.elapsed().as_secs();
        return keep(precheck::assemble(input, &out), &out);
    };
    let config = precheck::Config::of(&map);
    let mut touched = precheck::touched(&map, &flows_rel, &changed);
    // Unmapped files that draw nothing (helpers, type files) aren't held against the change.
    (touched.unmapped, touched.quiet) = precheck::split_unmapped(std::mem::take(&mut touched.unmapped), |f| read_changed(&root, f));
    input.unmapped_large = config.unmapped_large();
    let widths = config.widths();
    let conditions = precheck::conditions(&widths, &precheck::SCHEMES);
    let screens: HashSet<(String, String)> = touched.screens.iter().map(|s| (s.flow.clone(), s.screen.clone())).collect();
    let mut states: Vec<flows::Shot> = flows::shots(&design).into_iter().filter(|s| screens.contains(&(s.flow.clone(), s.screen.clone()))).collect();
    // The most relevant first: screens whose own source changed, each one's default state before its others.
    precheck::prioritize(&mut states, &touched.source_hits, |s| (format!("{}/{}", s.flow, s.screen), s.state == "default"));
    let (kept, cut) = precheck::take_most_relevant(states, config.max_states());
    let states = kept;
    let mut left_out: Vec<String> = cut.iter().map(key_of).collect();
    let (by_command, rest): (Vec<flows::Shot>, Vec<flows::Shot>) = states.into_iter().partition(|s| s.command.is_some());
    let (by_url, neither): (Vec<flows::Shot>, Vec<flows::Shot>) = rest.into_iter().partition(|s| s.url.is_some());
    let mut taken: Vec<precheck::Taken> = Vec::new();
    let failed = |s: &flows::Shot, why: &str, taken: &mut Vec<precheck::Taken>| {
        for c in conditions.iter().filter(|c| c.variant == precheck::Variant::Normal) {
            taken.push(precheck::Taken { key: key_of(s), condition: c.code(), error: Some(why.to_owned()), ..precheck::Taken::default() });
        }
    };
    for s in &neither {
        failed(s, "neither a url nor a command reaches it", &mut taken);
    }
    let mut preview = None;
    if !by_url.is_empty() {
        let origin = match (&r.url, &config.preview) {
            (Some(url), _) => Some(url.clone()),
            (None, Some(command)) => match Preview::start(command, &root, config.ready.as_deref(), &out).await {
                Ok(p) => {
                    let origin = p.origin.clone();
                    preview = Some(p);
                    Some(origin)
                }
                Err(why) => {
                    errors.push(format!("the preview didn't start: {why}"));
                    None
                }
            },
            (None, None) => {
                errors.push("its states are shot by URL, and nothing serves this worktree's code: give the map a precheck.preview, or --url".into());
                None
            }
        };
        match origin {
            Some(origin) => {
                let axe = match crate::design_system::axe().await {
                    Ok(path) => Some(path),
                    Err(why) => {
                        input.axe_skipped = Some(why);
                        None
                    }
                };
                match render(&by_url, &conditions, &config, &origin, &out, axe.as_deref()).await {
                    Ok(got) => taken.extend(got),
                    Err(why) => {
                        errors.push(format!("rendering stopped: {why}"));
                        for s in &by_url {
                            failed(s, &why, &mut taken);
                        }
                    }
                }
            }
            None => {
                for s in &by_url {
                    failed(s, "not rendered: no preview of this worktree", &mut taken);
                }
            }
        }
    }
    // A state shot by its command: light and dark (and the map's other modes) at each width, at normal size.
    let modes: Vec<String> = precheck::SCHEMES.iter().map(|m| (*m).to_owned()).chain(config.modes.iter().cloned()).collect();
    let origin = preview.as_ref().map(|p| p.origin.clone()).or_else(|| r.url.clone());
    for s in &by_command {
        // Past the time budget no state is started: it is left out whole, told as such, not half shot.
        if started.elapsed() > r.budget {
            left_out.push(key_of(s));
            continue;
        }
        for &width in &widths {
            for mode in &modes {
                let condition = precheck::Condition::new(width, mode, precheck::Variant::Normal).code();
                let path = out.join("shots").join(&s.flow).join(&s.screen).join(flows::picture_name(&s.state, mode, width));
                let error = shoot_by_command(s, &root, &path, width, mode, origin.as_deref()).await.err();
                taken.push(precheck::Taken { key: key_of(s), condition, path: error.is_none().then(|| path.to_string_lossy().into_owned()), dpr: 0., error, dom: None });
            }
        }
    }
    if let Some(p) = preview {
        p.stop().await;
    }
    // The map's pictures to compare with: at the base, and on the branch.
    for t in taken.iter().filter(|t| t.error.is_none() && t.condition.ends_with(".normal")) {
        let Some(c) = precheck::Condition::parse(&t.condition) else { continue };
        let mut parts = t.key.splitn(3, '/');
        let (Some(flow), Some(screen), Some(state)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let rel = format!("{flows_rel}/{flow}/{screen}/{}", flows::picture_name(state, &c.scheme, c.width));
        if let Some(bytes) = base.as_deref().and_then(|b| git_file(&root, b, &rel)) {
            let at = out.join("base").join(flow).join(screen).join(flows::picture_name(state, &c.scheme, c.width));
            if at.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&at, bytes).is_ok() {
                input.baselines.insert((t.key.clone(), t.condition.clone()), at);
            }
        }
        let committed = root.join(&rel);
        if committed.is_file() {
            input.committed.insert((t.key.clone(), t.condition.clone()), committed);
        }
    }
    input.touched = touched;
    input.left_out = left_out.len();
    input.left_out_states = left_out;
    input.conditions = conditions;
    input.taken = taken;
    input.errors = errors;
    input.secs = started.elapsed().as_secs();
    keep(precheck::assemble(input, &out), &out)
}

/// `report` kept in `out`, under what the gate would make of it.
fn keep(report: precheck::Report, out: &Path) -> Result<precheck::Report, String> {
    let call = design_qa::gate(Some(&report));
    precheck::write(out, &report, &report.summary(&call.headline(false))).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(report)
}

/// The states shot by URL rendered at every condition by `precheck.mjs`, on
/// `origin` (the preview); what each measured.
async fn render(states: &[flows::Shot], conditions: &[precheck::Condition], config: &precheck::Config, origin: &str, out: &Path, axe: Option<&Path>) -> Result<Vec<precheck::Taken>, String> {
    let mut plan = Vec::new();
    for s in states {
        let Some(url) = &s.url else { continue };
        let url = config.rebase(url, origin);
        for c in conditions {
            let mut take = serde_json::to_value(&s.capture).unwrap_or_else(|_| json!({}));
            let path = out.join("shots").join(&s.flow).join(&s.screen).join(format!("{}.{}.{}.{}.png", s.state, c.scheme, c.width, c.variant.code()));
            if let Value::Object(fields) = &mut take {
                fields.insert("key".into(), json!(key_of(s)));
                fields.insert("condition".into(), json!(c.code()));
                fields.insert("url".into(), json!(url));
                fields.insert("path".into(), json!(path));
                fields.insert("width".into(), json!(c.width));
                fields.insert("scheme".into(), json!(c.scheme));
                fields.insert("variant".into(), json!(c.variant.code()));
            }
            plan.push(take);
        }
    }
    eprintln!("{}", t!("design.precheck.rendering", states = states.len(), conditions = conditions.len(), pictures = plan.len()));
    let plan_file = out.join("plan.json");
    let results = out.join("measured.json");
    std::fs::write(&plan_file, serde_json::to_string(&plan).unwrap_or_default()).map_err(|e| e.to_string())?;
    let kit = crate::design::ensure_kit().await?;
    let (node, _) = crate::design::node_tools()?;
    let mut command = tokio::process::Command::new(&node);
    command.arg(kit.join("precheck.mjs")).arg("--plan").arg(&plan_file).arg("--json").arg(&results).args(["--jobs", "3"]).stdin(Stdio::null());
    if let Some(axe) = axe {
        command.arg("--axe").arg(axe);
    }
    let status = command.status().await.map_err(|e| format!("node: {e}"))?;
    if !status.success() {
        return Err(format!("precheck.mjs stopped ({status})"));
    }
    let text = std::fs::read_to_string(&results).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// One picture of a state by the map's command, at the repository's root,
/// given the preview's origin too (`{origin}`, `EQUIPE_SHOT_ORIGIN`).
async fn shoot_by_command(s: &flows::Shot, root: &Path, path: &Path, width: u32, mode: &str, origin: Option<&str>) -> Result<(), String> {
    let Some(command) = &s.command else { return Err("no command".into()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut values = flows::shot_values(path, mode, width, s);
    if let Some(origin) = origin {
        values.push(("origin", origin.to_owned()));
    }
    crate::design::made_by(&flows::fill(command, &values), path, root, &flows::env_of("EQUIPE_SHOT_", &values), Duration::from_secs(120), crate::design::Want::Png).await
}

pub async fn precheck_command(a: PrecheckArgs) -> Result<(), String> {
    let report = run(&Run { worktree: a.worktree, base: a.base, out: a.out.clone(), url: a.url, ticket: a.ticket, budget: Duration::from_secs(a.budget_secs) }).await?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
        return Ok(());
    }
    let call = design_qa::gate(Some(&report));
    println!("{}", t!("design.precheck.decision", decision = decision_word(call.decision), why = call.reasons.join(" ")));
    println!("{}", t!("design.precheck.wrote", path = a.out.join(precheck::REPORT_MD).display()));
    Ok(())
}

fn decision_word(d: Decision) -> &'static str {
    match d {
        Decision::AutoPass => t!("design.qa.decision.auto_pass"),
        Decision::SendBack => t!("design.qa.decision.send_back"),
        Decision::Designer => t!("design.qa.decision.designer"),
    }
}

// ── stats ──

fn squad_db(squad: Option<String>) -> Result<(String, PathBuf), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, squad).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    let db = equipe_core::journal::squad_dir(&home, &squad).join("squad.db");
    if !db.is_file() {
        return Err(t!("design.qa.no_journal", squad = squad));
    }
    Ok((squad, db))
}

/// A token count as people read it: 640k, 12.3M.
fn amount(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.0}k", n as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.2}B", n as f64 / 1e9),
    }
}

fn rate(r: Option<f64>) -> String {
    r.map_or_else(|| "—".to_owned(), |r| format!("{r:.0}%"))
}

fn per(total: u64, n: u32) -> u64 {
    if n == 0 { 0 } else { total / u64::from(n) }
}

fn row_line(name: &str, s: &qa_stats::Sums) -> String {
    t!("design.qa_stats.row", name = name, count = s.tickets, rate = rate(s.pass_rate()), per = amount(s.per_ticket()), total = amount(s.tokens), calls = per(s.calls, s.tickets), minutes = per(s.secs, s.tickets) / 60)
}

/// What `qa-stats` prints.
pub(crate) fn stats_text(squad: &str, since: &str, s: &qa_stats::Stats) -> String {
    if s.all.tickets == 0 {
        return t!("design.qa_stats.none", squad = squad, since = since);
    }
    let a = &s.all;
    let mut out = vec![
        t!("design.qa_stats.heading", squad = squad, since = since, count = a.tickets),
        t!("design.qa_stats.verdicts", passed = a.passed, changes = a.changes, cancelled = a.cancelled, open = a.open, rate = rate(a.pass_rate())),
        t!("design.qa_stats.tokens", total = amount(a.tokens), per = amount(a.per_ticket()), read = amount(a.cache_read), write = amount(a.cache_write), input = amount(a.input), output = amount(a.output)),
        t!("design.qa_stats.work", turns = a.turns, calls = a.calls, calls_per = per(a.calls, a.tickets), minutes = a.secs / 60),
        t!(
            "design.qa_stats.rounds",
            first = s.rounds.get(&1).copied().unwrap_or(0),
            second = s.rounds.get(&2).copied().unwrap_or(0),
            more = s.rounds.get(&3).copied().unwrap_or(0)
        ),
        t!("design.qa_stats.by_tier").to_owned(),
    ];
    out.extend(s.by_tier.iter().map(|(tier, sums)| row_line(tier, sums)));
    out.push(t!("design.qa_stats.by_gate").to_owned());
    out.extend(s.by_gate.iter().map(|(gate, sums)| row_line(gate, sums)));
    if !s.by_judgment.is_empty() {
        out.push(t!("design.qa_stats.by_judgment").to_owned());
        out.extend(s.by_judgment.iter().map(|(why, sums)| row_line(why, sums)));
    }
    let g = &s.agreement;
    let judged = g.pass_agreed + g.pass_missed + g.back_agreed + g.back_wrong;
    if judged + g.deferred > 0 {
        out.push(t!("design.qa_stats.agreement", rate = rate(g.rate()), judged = judged, pa = g.pass_agreed, pm = g.pass_missed, ba = g.back_agreed, bw = g.back_wrong, deferred = g.deferred));
    }
    if g.acted_pass + g.acted_back > 0 {
        out.push(t!("design.qa_stats.acted", pass = g.acted_pass, back = g.acted_back));
    }
    out.join("\n")
}

pub fn qa_stats_command(a: QaStatsArgs) -> Result<(), String> {
    let (squad, db) = squad_db(a.squad)?;
    let since = qa_stats::since(&a.since, now()).ok_or_else(|| t!("design.qa.bad_since", since = a.since))?;
    let rows = qa_stats::read(&db, since)?;
    let stats = qa_stats::summarize(&rows);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "squad": squad, "since": since, "stats": stats, "tickets": rows })).unwrap_or_default());
    } else {
        println!("{}", stats_text(&squad, &a.since, &stats));
    }
    Ok(())
}

pub fn qa_config_command(a: QaConfigArgs) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, a.squad).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    let journal = equipe_core::journal::Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    let gate = a.gate.as_deref().map(|g| GateMode::parse(g).ok_or_else(|| t!("design.qa_config.bad_gate").to_owned())).transpose()?;
    let tiers = a.tiers.as_deref().map(|p| TierPolicy::parse(p).ok_or_else(|| t!("design.qa_config.bad_tiers").to_owned())).transpose()?;
    if let Some(gate) = gate {
        journal.set_design_qa_gate(gate).map_err(|e| e.to_string())?;
    }
    if let Some(tiers) = tiers {
        journal.set_design_qa_tiers(tiers).map_err(|e| e.to_string())?;
    }
    let (now_gate, now_tiers) = (journal.design_qa_gate().code(), journal.design_qa_tiers().code());
    if gate.is_some() || tiers.is_some() {
        println!("{}", t!("design.qa_config.set", squad = squad, gate = now_gate, tiers = now_tiers));
    } else {
        println!("{}", t!("design.qa_config.now", squad = squad, gate = now_gate, tiers = now_tiers));
    }
    Ok(())
}

// ── replay ──

/// A past design QA ticket replayed.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Replayed {
    qa: u64,
    checked: Option<u64>,
    round: u32,
    /// What its designer decided then.
    recorded: String,
    recorded_tokens: u64,
    head: Option<String>,
    base: Option<String>,
    decision: Option<String>,
    judgment: Option<String>,
    /// `agree`, `missed` (would pass, it was sent back), `wrong` (would send back, it passed), `deferred`.
    agreement: Option<String>,
    report: Option<String>,
    designer: Option<DesignerRun>,
    skipped: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct DesignerRun {
    model: String,
    verdict: Option<String>,
    tokens: u64,
    turns: u64,
    error: Option<String>,
}

/// The commit design QA of `checked` looked at, handed in by `created`: the
/// latest of its commits (`equipe: T-<checked> …`) made by then.
fn handed_in_commit(repo: &Path, checked: u64, created: i64) -> Option<String> {
    let log = git(repo, &["log", "--all", "--format=%H %ct", &format!("--grep=^equipe: T-{checked} "), "-n", "200"]).ok()?;
    log.lines().filter_map(|l| l.split_once(' ')).filter_map(|(h, t)| t.trim().parse::<i64>().ok().map(|t| (h, t))).filter(|(_, t)| *t <= created + 300).max_by_key(|(_, t)| *t).map(|(h, _)| h.to_owned())
}

/// Where `head` met the main branch as it stood at `created`.
fn base_then(repo: &Path, head: &str, created: i64) -> Option<String> {
    let main = equipe_core::worktree::main_branch(repo).ok()?;
    let then = git(repo, &["rev-list", "-1", "--first-parent", &format!("--before={created}"), &main]).ok().filter(|s| !s.is_empty())?;
    git(repo, &["merge-base", head, &then]).ok()
}

fn agreement(decision: Decision, verdict: &str) -> &'static str {
    match (decision, verdict) {
        (Decision::AutoPass, "pass") | (Decision::SendBack, "changes") => "agree",
        (Decision::AutoPass, _) => "missed",
        (Decision::SendBack, _) => "wrong",
        (Decision::Designer, _) => "deferred",
    }
}

pub async fn qa_replay_command(a: QaReplayArgs) -> Result<(), String> {
    let (squad, db) = squad_db(a.squad.clone())?;
    let since = qa_stats::since(&a.since, now()).ok_or_else(|| t!("design.qa.bad_since", since = a.since))?;
    let mut rows: Vec<qa_stats::QaRow> = qa_stats::read(&db, since)?.into_iter().filter(|r| matches!(r.verdict.as_str(), "pass" | "changes")).collect();
    if !a.tickets.is_empty() {
        rows.retain(|r| a.tickets.contains(&r.qa));
    }
    let skip = rows.len().saturating_sub(a.last.max(1));
    rows.drain(..skip);
    if rows.is_empty() {
        println!("{}", t!("design.qa_replay.none"));
        return Ok(());
    }
    let out = a.out.clone().unwrap_or_else(|| equipe_providers::credentials::equipe_home().join("design-qa-replay").join(now().to_string()));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let out = std::path::absolute(&out).unwrap_or(out);
    if a.dry_run {
        eprintln!("{}", t!("design.qa_replay.dry"));
    }
    let mut done: Vec<Replayed> = Vec::new();
    for row in rows {
        let mut r = Replayed { qa: row.qa, checked: row.checked, round: row.round, recorded: row.verdict.clone(), recorded_tokens: row.tokens(), ..Replayed::default() };
        match replay_one(&db, &row, &a, &out, &mut r).await {
            Ok(()) => {}
            Err(why) => r.skipped = Some(why),
        }
        match &r.skipped {
            Some(why) => eprintln!("{}", t!("design.qa_replay.skipped", qa = r.qa, why = why)),
            None => {
                let decision = r.decision.as_deref().and_then(Decision::parse).map_or("—", decision_word);
                let agree = r.agreement.as_deref().map(|a| format!(" · {a}")).unwrap_or_default();
                println!("{}", t!("design.qa_replay.ticket", qa = r.qa, checked = r.checked.unwrap_or(0), round = r.round, verdict = r.recorded, decision = decision, agree = agree));
                if let Some(d) = &r.designer {
                    println!("{}", t!("design.qa_replay.designer", model = d.model, verdict = d.verdict.as_deref().unwrap_or("—"), tokens = amount(d.tokens), turns = d.turns));
                }
            }
        }
        done.push(r);
    }
    let replayed: Vec<&Replayed> = done.iter().filter(|r| r.skipped.is_none()).collect();
    let count = |what: &str, decision: &str| replayed.iter().filter(|r| r.agreement.as_deref() == Some(what) && r.decision.as_deref() == Some(decision)).count();
    let (pa, pm, ba, bw) = (count("agree", "auto_pass"), count("missed", "auto_pass"), count("agree", "send_back"), count("wrong", "send_back"));
    let deferred = replayed.iter().filter(|r| r.agreement.as_deref() == Some("deferred")).count();
    let judged = pa + pm + ba + bw;
    let agreed = (judged > 0).then(|| (pa + ba) as f64 * 100. / judged as f64);
    println!("{}", t!("design.qa_replay.summary", count = replayed.len(), skipped = done.len() - replayed.len(), rate = rate(agreed), pa = pa, pm = pm, ba = ba, bw = bw, deferred = deferred));
    if !a.dry_run {
        let recorded: u64 = replayed.iter().map(|r| r.recorded_tokens).sum();
        let new: u64 = replayed.iter().filter_map(|r| r.designer.as_ref()).map(|d| d.tokens).sum();
        println!("{}", t!("design.qa_replay.tokens", count = replayed.len(), recorded = amount(recorded), replayed = amount(new)));
    }
    let kept = out.join("replay.json");
    let _ = std::fs::write(&kept, serde_json::to_string_pretty(&json!({ "squad": squad, "dry_run": a.dry_run, "tickets": done })).unwrap_or_default());
    if a.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "squad": squad, "dry_run": a.dry_run, "tickets": done })).unwrap_or_default());
    }
    println!("{}", t!("design.qa_replay.kept", path = kept.display()));
    Ok(())
}

/// One past design QA ticket through the precheck (and the designer), in a
/// temporary worktree of the commit it checked, removed after.
async fn replay_one(db: &Path, row: &qa_stats::QaRow, a: &QaReplayArgs, out: &Path, r: &mut Replayed) -> Result<(), String> {
    let checked_id = row.checked.ok_or("its record doesn't say which ticket it checked")?;
    let checked = qa_stats::checked(db, checked_id)?.ok_or_else(|| format!("T-{checked_id} isn't on the board"))?;
    let project = checked.project.clone().filter(|p| p.is_dir()).ok_or_else(|| format!("T-{checked_id}'s project isn't on this machine"))?;
    let repo = equipe_core::worktree::repo_root(&project)?;
    let (kept_head, kept_base) = qa_stats::commits(db, row.qa);
    let head = kept_head.or_else(|| handed_in_commit(&repo, checked_id, row.created)).ok_or_else(|| format!("no commit of T-{checked_id} from before its design QA"))?;
    let base = kept_base.or_else(|| base_then(&repo, &head, row.created));
    r.head = Some(head.clone());
    r.base = base.clone();
    let wt = out.join("worktrees").join(format!("T-{}", row.qa));
    let _ = std::fs::remove_dir_all(&wt);
    git(&repo, &["worktree", "add", "--detach", "--quiet", &wt.to_string_lossy(), &head])?;
    // Its dependencies as the live worktrees get them: a copy-on-write clone.
    let seeded = wt.clone();
    let _ = tokio::task::spawn_blocking(move || equipe_core::worktree::seed_dependencies(&seeded)).await;
    let dir = out.join(format!("T-{}", row.qa));
    let result = run(&Run { worktree: wt.clone(), base: base.clone().or_else(|| equipe_core::worktree::main_branch(&repo).ok()), out: dir.clone(), url: None, ticket: Some(checked_id), budget: Duration::from_secs(a.budget_secs) }).await;
    let call = design_qa::gate(result.as_ref().ok());
    r.decision = Some(call.decision.code().to_owned());
    r.judgment = call.judgment.map(|j| j.code().to_owned());
    r.agreement = Some(agreement(call.decision, &row.verdict).to_owned());
    r.report = result.is_ok().then(|| dir.join(precheck::REPORT_MD).to_string_lossy().into_owned());
    if !a.dry_run {
        r.designer = Some(replay_designer(&wt, &dir, &checked, base.as_deref(), &a.designer_model, &a.designer_effort).await);
    }
    let _ = git(&repo, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&wt);
    let _ = git(&repo, &["worktree", "prune"]);
    result.map(|_| ())
}

/// A cheaper-tier designer on the replayed ticket, given the precheck's
/// report as design QA now gives it, through Claude Code in print mode —
/// reading only: the worktree, the report and its pictures, and git.
async fn replay_designer(wt: &Path, dir: &Path, checked: &qa_stats::Checked, base: Option<&str>, model: &str, effort: &str) -> DesignerRun {
    let mut run = DesignerRun { model: format!("{model} · {effort}"), ..DesignerRun::default() };
    let summary = std::fs::read_to_string(dir.join(precheck::REPORT_MD)).unwrap_or_else(|_| "The precheck wrote no report.".into());
    let diff = base.map(|b| format!("`git diff {b}...HEAD`")).unwrap_or_else(|| "`git log -5 -p`".into());
    let prompt = format!(
        "You are replaying a design QA check for measurement. The worktree here is the work as it was handed in (read it, never change it).\n\n\
What the ticket asked ({title}):\n{body}\n\nequipe's precheck report (pictures under {dir}):\n{summary}\n\n\
Start from the report: look at its flagged pictures with Read, and at the change with {diff}. Don't render anything. Decide as design QA does: \
blockers (S1/S2) — function or data broken, layout broken at a major width, a WCAG 2.2 A/AA failure, a clear difference from what was asked — \
send it back; polish (S3/S4) never does. End your answer with one line of JSON and nothing after it: \
{{\"verdict\": \"pass\" or \"changes\", \"blockers\": [\"…\"]}}",
        title = checked.title,
        body = checked.body.trim(),
        dir = dir.display(),
    );
    let output = tokio::process::Command::new(equipe_providers::claude_code::DEFAULT_BINARY)
        .args(["-p", &prompt, "--model", model, "--effort", effort, "--output-format", "json", "--permission-mode", "dontAsk", "--no-session-persistence", "--strict-mcp-config"])
        .args(["--allowedTools", "Read,Glob,Grep,Bash(git diff:*),Bash(git log:*),Bash(git show:*)"])
        .arg("--add-dir")
        .arg(dir)
        .current_dir(wt)
        .stdin(Stdio::null())
        .output()
        .await;
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            run.error = Some(e.to_string());
            return run;
        }
    };
    let said: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let usage = &said["usage"];
    let n = |k: &str| usage[k].as_u64().unwrap_or(0);
    run.tokens = n("input_tokens") + n("output_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    run.turns = said["num_turns"].as_u64().unwrap_or(0);
    let result = said["result"].as_str().unwrap_or_default();
    run.verdict = result.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).and_then(|v| v["verdict"].as_str().map(str::to_owned));
    if !output.status.success() {
        run.error = Some(String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or("claude stopped").to_owned());
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        action: crate::design::DesignAction,
    }

    #[test]
    fn the_design_qa_commands_take_their_flags() {
        let parsed = Cli::try_parse_from(["x", "precheck", "--worktree", "/w", "--base", "main", "--out", "/o", "--ticket", "4"]).unwrap();
        let crate::design::DesignAction::Precheck(p) = parsed.action else { panic!("precheck") };
        assert_eq!((p.worktree, p.base.as_deref(), p.out, p.ticket, p.budget_secs), (PathBuf::from("/w"), Some("main"), PathBuf::from("/o"), Some(4), 900));
        let parsed = Cli::try_parse_from(["x", "qa-replay", "--tickets", "2077,2065", "--dry-run"]).unwrap();
        let crate::design::DesignAction::QaReplay(r) = parsed.action else { panic!("qa-replay") };
        assert_eq!((r.tickets, r.dry_run, r.designer_model.as_str(), r.last), (vec![2077, 2065], true, "sonnet", 20));
        let parsed = Cli::try_parse_from(["x", "qa-stats", "--since", "7d", "--json"]).unwrap();
        let crate::design::DesignAction::QaStats(s) = parsed.action else { panic!("qa-stats") };
        assert_eq!((s.since.as_str(), s.json), ("7d", true));
        assert!(Cli::try_parse_from(["x", "qa-config", "--gate", "on", "--tiers", "economy"]).is_ok());
    }

    #[test]
    fn stats_print_counts_pass_rate_agreement_and_tokens_by_tier() {
        let row = |qa: u64, verdict: &str, gate: Option<&str>, decision: Option<&str>, tier: &str, tokens: u64| qa_stats::QaRow {
            qa,
            verdict: verdict.into(),
            gate: gate.map(str::to_owned),
            decision: decision.map(str::to_owned),
            tier: Some(tier.into()),
            cache_read: tokens,
            turns: 2,
            calls: 70,
            secs: 1200,
            round: 1,
            ..qa_stats::QaRow::default()
        };
        let stats = qa_stats::summarize(&[
            row(1, "pass", None, None, "deep", 6_400_000),
            row(2, "changes", Some("shadow"), Some("send_back"), "standard", 900_000),
            row(3, "pass", Some("shadow"), Some("auto_pass"), "standard", 700_000),
            qa_stats::QaRow { judgment: Some("capped".into()), ..row(4, "pass", Some("shadow"), Some("designer"), "standard", 800_000) },
        ]);
        let text = stats_text("acme", "7d", &stats);
        assert!(text.contains("acme") && text.contains('4'), "{text}");
        assert!(text.contains("75%"), "pass rate 3 of 4: {text}");
        assert!(text.contains("6.4M") && text.contains("deep") && text.contains("standard"), "tokens by tier: {text}");
        assert!(text.contains("100%"), "the gate agreed with both verdicts: {text}");
        assert!(text.contains(t!("design.qa_stats.by_judgment")) && text.contains(&row_line("capped", &stats.by_judgment["capped"])) && stats.by_judgment.contains_key("send_back"), "a count per judgment reason: {text}");
        assert_eq!(stats_text("acme", "7d", &qa_stats::Stats::default()), t!("design.qa_stats.none", squad = "acme", since = "7d"));
        assert_eq!((amount(950), amount(640_000), amount(12_300_000)), ("950".to_owned(), "640k".to_owned(), "12.3M".to_owned()));
    }

    #[test]
    fn a_replay_compares_the_gate_with_what_the_designer_decided() {
        assert_eq!(agreement(Decision::AutoPass, "pass"), "agree");
        assert_eq!(agreement(Decision::AutoPass, "changes"), "missed");
        assert_eq!(agreement(Decision::SendBack, "pass"), "wrong");
        assert_eq!(agreement(Decision::Designer, "changes"), "deferred");
    }

    #[test]
    fn a_precheck_over_the_cap_shoots_the_most_relevant_state_and_names_what_it_left_out() {
        let dir = std::env::temp_dir().join(format!("equipe-precheck-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(repo.join("design/flows")).unwrap();
        std::fs::create_dir_all(repo.join("src/own")).unwrap();
        std::fs::create_dir_all(repo.join("src/many")).unwrap();
        let state = |id: &str| serde_json::json!({ "id": id, "command": "false" });
        let map = serde_json::json!({ "precheck": { "maxStates": 1 }, "flows": [ { "id": "f", "screens": [
            { "id": "untouched", "source": ["src/nowhere/"], "states": [state("default")] },
            { "id": "own", "source": ["src/own/"], "states": [state("error"), state("default")] },
            { "id": "many", "source": ["src/many/"], "states": [state("default")] } ] } ] });
        std::fs::write(repo.join("design/flows/flows.json"), map.to_string()).unwrap();
        let g = |args: &[&str]| assert!(git_cmd(&repo).args(args).output().unwrap().status.success(), "{args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["add", "."]);
        g(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "-m", "first"]);
        std::fs::write(repo.join("src/own/a.css"), "a{}").unwrap();
        for n in 0..3 {
            std::fs::write(repo.join(format!("src/many/{n}.css")), "a{}").unwrap();
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report = rt.block_on(run(&Run { worktree: repo.clone(), base: Some("main".into()), out: dir.join("out"), url: None, ticket: Some(3), budget: Duration::from_secs(60) })).unwrap();
        assert_eq!(report.touched, ["f/many", "f/own"]);
        assert!(report.shots.iter().all(|s| s.key == "f/many/default"), "the screen with the most changed source files, its default state: {:?}", report.shots.iter().map(|s| &s.key).collect::<Vec<_>>());
        assert_eq!((report.left_out, report.left_out_states), (2, vec!["f/own/default".to_owned(), "f/own/error".to_owned()]));
        // Out of time before the first state: all are left out and the report says nothing was rendered.
        let late = rt.block_on(run(&Run { worktree: repo.clone(), base: Some("main".into()), out: dir.join("late"), url: None, ticket: Some(3), budget: Duration::ZERO })).unwrap();
        assert_eq!((late.shots.len(), late.left_out), (0, 3));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_precheck_of_a_worktree_without_a_map_says_so_and_leaves_it_to_a_designer() {
        let dir = std::env::temp_dir().join(format!("equipe-precheck-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let g = |args: &[&str]| assert!(git_cmd(&repo).args(args).output().unwrap().status.success(), "{args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        std::fs::write(repo.join("src/page.tsx"), "export default () => <main>Hi</main>;\n").unwrap();
        std::fs::write(repo.join("src/format.tsx"), "export const format = (n: number) => n.toFixed(2);\n").unwrap();
        let out = dir.join("out");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report = rt.block_on(run(&Run { worktree: repo.clone(), base: Some("main".into()), out: out.clone(), url: None, ticket: Some(3), budget: Duration::from_secs(5) })).unwrap();
        assert!(report.errors.iter().any(|e| e.contains("no UI map")), "{:?}", report.errors);
        assert_eq!(report.unmapped, ["src/page.tsx"], "an uncommitted UI file counts");
        assert_eq!(report.unmapped_quiet, ["src/format.tsx"], "a .tsx that draws nothing doesn't");
        let call = design_qa::gate(Some(&report));
        assert_eq!((call.decision, call.judgment), (Decision::Designer, Some(design_qa::Judgment::PrecheckFailed)));
        assert!(out.join(precheck::REPORT_JSON).is_file() && out.join(precheck::REPORT_MD).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }
}
