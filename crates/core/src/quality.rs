//! How well members behave, read from what they did — for the quality gate
//! (docs/quality-gate.md), never by the daemon.
//!
//! - [`Transcript`]: a Claude Code session's transcript (its JSONL file) read
//!   into [`Step`]s, with the tokens it spent and the cost Claude Code put
//!   on it.
//! - [`skill_checks`]: the situations that call for one of equipe's skills —
//!   writing a plan, a UI file, an `equipe design` command — and whether the
//!   member opened that skill before, right after, later or never.
//! - [`Rules`]: what a member's steps show of the rules equipe gives it — a
//!   foreground `sleep`, `equipe browser`, `estimate`, `ask_decision` with a
//!   recommendation, the engine's own question tool, the guard's refusals.
//!
//! The behaviour eval (`crates/cli/tests/quality_gate`) grades its runs with
//! these; `crates/core/examples/quality_metrics.rs` reads a squad's real
//! journal and transcripts with them. Both only read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub mod live;

/// One thing that happened in a member's session, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A user turn's text: the person, or equipe on their behalf.
    User(String),
    /// The agent's own words.
    Reply(String),
    /// A tool call, with its whole input.
    Tool { id: String, name: String, input: Value },
    /// A tool call's answer: whether it failed or was refused, and its text.
    Answer { id: String, error: bool, text: String },
}

/// Tokens a session spent, as the API counted them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

impl Tokens {
    /// Everything processed: input, cache writes and reads, and output.
    pub fn total(&self) -> u64 {
        self.input + self.cache_write + self.cache_read + self.output
    }

    pub fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self.output += other.output;
    }
}

/// A Claude Code session's transcript, read.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub path: PathBuf,
    /// Claude Code's session id.
    pub session: String,
    /// Where its turns ran, as it first says.
    pub cwd: Option<String>,
    /// The git branch it first ran on.
    pub branch: Option<String>,
    /// A subagent's own transcript (`isSidechain`).
    pub sidechain: bool,
    /// The role it played, when equipe's paths say: `members/<role>/` in the
    /// instructions it was given, or a branch `equipe/<role>-t<n>…`.
    pub role: Option<String>,
    /// equipe's skills it was offered (`equipe:<name>` in its skill listing),
    /// and at which step each was first offered.
    pub offered: Vec<(String, usize)>,
    pub steps: Vec<Step>,
    pub tokens: Tokens,
    /// What Claude Code says the session cost at API prices, the most it
    /// reported (`cost-state`); a subscription isn't charged this.
    pub cost_usd: Option<f64>,
    /// When it started and last wrote, as Claude Code stamped its lines.
    pub first_at: Option<String>,
    pub last_at: Option<String>,
}

impl Transcript {
    /// Read a transcript file; lines it can't read are skipped.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut t = Self::parse(&text);
        t.path = path.to_owned();
        Ok(t)
    }

    /// A transcript's lines, read.
    pub fn parse(text: &str) -> Self {
        let mut t = Transcript::default();
        // Claude Code writes one line per content block, each with the
        // message's usage: count each message once, its last usage.
        let mut usage: HashMap<String, Tokens> = HashMap::new();
        let mut unnamed = Tokens::default();
        for line in text.lines() {
            let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
            let s = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_owned);
            if t.session.is_empty()
                && let Some(id) = s("sessionId")
            {
                t.session = id;
            }
            if t.cwd.is_none() {
                t.cwd = s("cwd");
            }
            if t.branch.is_none() {
                t.branch = s("gitBranch").filter(|b| !b.is_empty());
            }
            if d.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                t.sidechain = true;
            }
            if let Some(at) = s("timestamp") {
                t.first_at.get_or_insert_with(|| at.clone());
                t.last_at = Some(at);
            }
            if let Some(a) = d.get("attachment") {
                let content = a.get("content").map(|c| c.as_str().map(str::to_owned).unwrap_or_else(|| c.to_string())).unwrap_or_default();
                match a.get("type").and_then(Value::as_str) {
                    Some("skill_listing") => {
                        for name in equipe_skills_named(&content) {
                            if !t.offered.iter().any(|(n, _)| *n == name) {
                                t.offered.push((name, t.steps.len()));
                            }
                        }
                    }
                    Some("instructions") if t.role.is_none() => t.role = role_in(&a.to_string()),
                    _ => {}
                }
            }
            match d.get("type").and_then(Value::as_str) {
                Some("cost-state") => {
                    if let Some(cost) = d.get("totalCostUSD").and_then(Value::as_f64) {
                        t.cost_usd = Some(t.cost_usd.map_or(cost, |c: f64| c.max(cost)));
                    }
                }
                Some("user") => {
                    let content = d.pointer("/message/content");
                    match content {
                        Some(Value::String(text)) => t.steps.push(Step::User(text.clone())),
                        Some(Value::Array(blocks)) => {
                            for b in blocks {
                                match b.get("type").and_then(Value::as_str) {
                                    Some("text") => t.steps.push(Step::User(b.get("text").and_then(Value::as_str).unwrap_or_default().to_owned())),
                                    Some("tool_result") => t.steps.push(Step::Answer {
                                        id: b.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                                        error: b.get("is_error").and_then(Value::as_bool) == Some(true),
                                        text: text_of(b.get("content")),
                                    }),
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some("assistant") => {
                    let message = d.get("message").cloned().unwrap_or(Value::Null);
                    if let Some(u) = message.get("usage") {
                        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                        let tokens = Tokens { input: n("input_tokens"), cache_write: n("cache_creation_input_tokens"), cache_read: n("cache_read_input_tokens"), output: n("output_tokens") };
                        match message.get("id").and_then(Value::as_str) {
                            Some(id) => {
                                usage.insert(id.to_owned(), tokens);
                            }
                            None => unnamed.add(tokens),
                        }
                    }
                    for b in message.get("content").and_then(Value::as_array).into_iter().flatten() {
                        match b.get("type").and_then(Value::as_str) {
                            Some("text") => t.steps.push(Step::Reply(b.get("text").and_then(Value::as_str).unwrap_or_default().to_owned())),
                            Some("tool_use") => t.steps.push(Step::Tool {
                                id: b.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                                name: b.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                                input: b.get("input").cloned().unwrap_or(Value::Null),
                            }),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        t.tokens = unnamed;
        for tokens in usage.values() {
            t.tokens.add(*tokens);
        }
        if t.role.is_none() {
            t.role = t.branch.as_deref().and_then(role_of_branch);
        }
        t
    }

    /// Its tool calls, in order: name and input.
    pub fn tools(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.steps.iter().filter_map(|s| match s {
            Step::Tool { name, input, .. } => Some((name.as_str(), input)),
            _ => None,
        })
    }
}

/// Every transcript under `dir` (a Claude Code projects folder, or one
/// project's), subagents' included.
pub fn transcripts_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

/// Claude Code's projects folder: `$CLAUDE_CONFIG_DIR/projects`, else
/// `~/.claude/projects`. Only transcripts are read there.
pub fn claude_projects() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude/projects"))
}

/// How Claude Code names a working directory's folder of transcripts.
pub fn claude_project_dir_name(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect()
}

fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The names in `equipe:<name>` mentions, in order, once each.
fn equipe_skills_named(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (at, _) in text.match_indices("equipe:") {
        let name: String = text[at + "equipe:".len()..].chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
        let name = name.trim_end_matches('-').to_owned();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// `members/<role>/` in a path equipe gave the session.
fn role_in(text: &str) -> Option<String> {
    let at = text.find("members/")? + "members/".len();
    let role: String = text[at..].chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
    (!role.is_empty() && text[at + role.len()..].starts_with('/')).then_some(role)
}

/// The role in a ticket branch's name, `equipe/<role>-t<n>…`.
fn role_of_branch(branch: &str) -> Option<String> {
    let rest = branch.strip_prefix("equipe/")?;
    let (role, after) = rest.split_once("-t")?;
    (after.starts_with(|c: char| c.is_ascii_digit()) && role.chars().all(|c| c.is_ascii_lowercase())).then(|| role.to_owned())
}

/// Whether `step` opens the skill `skill`: Claude Code's Skill tool on
/// `equipe:<skill>`, a read of its file, or Claude Code's echo of it.
pub fn opens(step: &Step, skill: &str) -> bool {
    match step {
        Step::Tool { name, input, .. } if name == "Skill" => input.get("skill").and_then(Value::as_str).is_some_and(|s| s == format!("equipe:{skill}") || s == skill),
        Step::Tool { name, input, .. } if name == "Read" || name == "Bash" => {
            let text = format!("{}{}", input.get("file_path").and_then(Value::as_str).unwrap_or(""), input.get("command").and_then(Value::as_str).unwrap_or(""));
            names_skill_path(&text, skill)
        }
        Step::User(text) => text.contains("Base directory for this skill") && names_skill_path(text, skill),
        _ => false,
    }
}

/// `…/skills/<skill>` followed by `/` or the end of a word.
fn names_skill_path(text: &str, skill: &str) -> bool {
    let needle = format!("/skills/{skill}");
    text.match_indices(&needle).any(|(at, _)| text[at + needle.len()..].chars().next().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-')))
}

/// When a member opened a skill its situation called for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Opened {
    /// Before the step that called for it.
    Before,
    /// Within the few steps after it.
    RightAfter,
    /// Later than that.
    Later,
    Never,
}

impl Opened {
    /// Whether this counts as having followed the skill: opened before, or right after.
    pub fn in_time(self) -> bool {
        matches!(self, Opened::Before | Opened::RightAfter)
    }
}

/// A situation that called for a skill, and how the member answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCheck {
    pub skill: String,
    /// What called for it, shortened.
    pub trigger: String,
    pub opened: Opened,
}

/// Skills that answer for each other: having opened any counts.
fn alternatives(skill: &str) -> Vec<&str> {
    match skill {
        "design-tools" => vec!["design-tools", "ui-map", "design-qa", "ui-floor"],
        "ui-map" => vec!["ui-map", "design-tools"],
        other => vec![other],
    }
}

/// How many steps after its trigger a skill still counts as opened right after.
const RIGHT_AFTER: usize = 8;

/// UI files: what the UI floor is for.
fn is_ui_file(path: &str) -> bool {
    [".tsx", ".jsx", ".css", ".scss", ".html", ".vue", ".svelte", ".swift", ".kt"].iter().any(|x| path.ends_with(x))
}

/// The situations in `t` that called for one of the skills it was offered,
/// each skill once, at its first trigger: the PM writing a plan, a designer
/// starting on what it was given, a UI file edited, `equipe design` run, the
/// browser or the desktop used, a routine's message, a plan review.
/// `role` is the member's (else the transcript's own guess).
pub fn skill_checks(t: &Transcript, role: Option<&str>) -> Vec<SkillCheck> {
    let role = role.or(t.role.as_deref());
    let steps = &t.steps;
    let offered_at = |skill: &str| t.offered.iter().find(|(n, _)| n == skill).map(|(_, at)| *at);
    let mut seen: Vec<String> = Vec::new();
    let mut checks = Vec::new();
    let mut check = |skill: &str, at: usize, trigger: &str| {
        if seen.iter().any(|s| s == skill) {
            return;
        }
        let Some(since) = offered_at(skill) else { return };
        if at < since {
            return;
        }
        seen.push(skill.to_owned());
        let alts = alternatives(skill);
        let any = |range: &[Step]| range.iter().any(|s| alts.iter().any(|k| opens(s, k)));
        let opened = if any(&steps[..at]) {
            Opened::Before
        } else if any(&steps[at..(at + RIGHT_AFTER).min(steps.len())]) {
            Opened::RightAfter
        } else if any(steps) {
            Opened::Later
        } else {
            Opened::Never
        };
        checks.push(SkillCheck { skill: skill.to_owned(), trigger: trigger.chars().take(90).collect(), opened });
    };
    let first_user = steps.iter().position(|s| matches!(s, Step::User(text) if !text.starts_with('<')));
    for (i, step) in steps.iter().enumerate() {
        match step {
            Step::User(text) => {
                if text.contains("[Idea start") {
                    check("office-hours", i, text);
                }
                let opening = text.trim_start();
                if ["[Routine ·", "[Idle ·", "[Signal ·"].iter().any(|p| opening.starts_with(p)) || text.contains("[Retro facts") {
                    check("routines-and-signals", i, text);
                }
                let lower = text.to_lowercase();
                if ["review the plan", "review this plan", "review my plan", "plan review", "[plan review"].iter().any(|p| lower.contains(p)) {
                    check("plan-review", i, text);
                }
                if lower.contains("design qa") || lower.contains("design_qa") {
                    check("design-qa", i, text);
                }
            }
            Step::Tool { name, input, .. } => {
                let command = if name == "Bash" { input.get("command").and_then(Value::as_str).unwrap_or("") } else { "" };
                if name.to_lowercase().contains("browser") || has_command(command, "equipe browser") {
                    check("browser", i, &format!("{name} {command}"));
                }
                if name.to_lowercase().contains("desktop") {
                    check("desktop-apps", i, name);
                }
                if name.ends_with("write_plan") {
                    check("planning", i, name);
                }
                if has_command(command, "equipe design") {
                    check("design-tools", i, command);
                }
                if name.ends_with("design_qa") {
                    check("design-qa", i, name);
                }
                let file = if ["Write", "Edit", "MultiEdit"].contains(&name.as_str()) { input.get("file_path").and_then(Value::as_str).unwrap_or("") } else { "" };
                if !file.is_empty() && is_ui_file(file) && matches!(role, Some("designer" | "frontend" | "mobile")) {
                    check("ui-floor", i, file);
                }
                if format!("{file}{command}").contains("design/flows") {
                    check("ui-map", i, if file.is_empty() { command } else { file });
                }
                if name.ends_with("create_ticket") && role == Some("pm") && input.to_string().to_lowercase().contains("design") {
                    check("working-with-design", i, input.get("title").and_then(Value::as_str).unwrap_or(""));
                }
            }
            _ => {}
        }
    }
    if let Some(at) = first_user {
        let Step::User(text) = &steps[at] else { unreachable!() };
        let lower = text.to_lowercase();
        match role {
            Some("designer") if !["qa", "plan review", "ui map", "since your last turn"].iter().any(|p| lower.contains(p)) => check("design-process", at, text),
            Some("security") => check("security-review", at, text),
            Some("marketing") => check("marketing-playbook", at, text),
            _ => {}
        }
    }
    checks
}

/// Whether a command line runs `words` as a command (not inside an echo's
/// text): at its start or after `;`, `&&`, `||`, `|`, `(`, or a path to it.
fn has_command(line: &str, words: &str) -> bool {
    let (program, rest) = words.split_once(' ').unwrap_or((words, ""));
    crate::guard::commands(line).iter().any(|w| {
        let first = w.first().map(String::as_str).unwrap_or("");
        let named = first == program || first.ends_with(&format!("/{program}"));
        named && (rest.is_empty() || w.get(1).map(String::as_str) == Some(rest))
    })
}

/// What a member's steps show of the rules it is given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub tool_calls: usize,
    /// `sleep`s the guard would refuse in the foreground — attempted, whether or not refused.
    pub foreground_sleeps: usize,
    /// `equipe browser` run by the member.
    pub equipe_browser: usize,
    /// Calls the guard refused (`Blocked: …`).
    pub guard_refusals: usize,
    pub estimates: usize,
    /// `ask_decision` calls, and those with an option marked `recommended`
    /// (of those whose questions have options at all).
    pub decisions: usize,
    pub decisions_with_options: usize,
    pub decisions_recommended: usize,
    /// A "(Recommended)" written into an option's label, which equipe marks itself.
    pub recommended_in_label: usize,
    /// Claude Code's own question tool, which no one sees.
    pub engine_questions: usize,
    /// Replies that end on a numbered list of questions instead of a tool.
    pub prose_questions: usize,
    pub submits: usize,
    pub tickets_created: usize,
}

impl Rules {
    pub fn add(&mut self, o: &Rules) {
        self.tool_calls += o.tool_calls;
        self.foreground_sleeps += o.foreground_sleeps;
        self.equipe_browser += o.equipe_browser;
        self.guard_refusals += o.guard_refusals;
        self.estimates += o.estimates;
        self.decisions += o.decisions;
        self.decisions_with_options += o.decisions_with_options;
        self.decisions_recommended += o.decisions_recommended;
        self.recommended_in_label += o.recommended_in_label;
        self.engine_questions += o.engine_questions;
        self.prose_questions += o.prose_questions;
        self.submits += o.submits;
        self.tickets_created += o.tickets_created;
    }
}

/// The equipe board tool `name` is, under any server prefix (`mcp__equipe__estimate`).
pub fn board_tool(name: &str) -> &str {
    name.rsplit("__").next().unwrap_or(name)
}

/// Read the rules out of a session's steps.
pub fn rules(steps: &[Step]) -> Rules {
    let mut r = Rules::default();
    let nowhere = Path::new("/");
    for step in steps {
        match step {
            Step::Tool { name, input, .. } => {
                r.tool_calls += 1;
                if name == "Bash" {
                    let command = input.get("command").and_then(Value::as_str).unwrap_or("");
                    if has_command(command, "equipe browser") {
                        r.equipe_browser += 1;
                    } else if crate::guard::refusal("Bash", input, nowhere, nowhere, &[]).is_some() {
                        r.foreground_sleeps += 1;
                    }
                }
                if name == "AskUserQuestion" {
                    r.engine_questions += 1;
                }
                if !name.starts_with("mcp__") {
                    continue;
                }
                match board_tool(name) {
                    "estimate" => r.estimates += 1,
                    "submit_for_review" => r.submits += 1,
                    "create_ticket" => r.tickets_created += 1,
                    "ask_decision" => {
                        r.decisions += 1;
                        let options: Vec<&Value> = input.get("questions").and_then(Value::as_array).into_iter().flatten().flat_map(|q| q.get("options").and_then(Value::as_array).into_iter().flatten()).collect();
                        if !options.is_empty() {
                            r.decisions_with_options += 1;
                            if options.iter().any(|o| o.get("recommended").and_then(Value::as_bool) == Some(true)) {
                                r.decisions_recommended += 1;
                            }
                        }
                        r.recommended_in_label += options.iter().filter(|o| o.get("label").and_then(Value::as_str).is_some_and(|l| equipe_protocol::decide::recommended(l).1)).count();
                    }
                    _ => {}
                }
            }
            Step::Answer { error: true, text, .. } if text.contains("Blocked: ") => r.guard_refusals += 1,
            Step::Reply(text) if ends_on_numbered_questions(text) => r.prose_questions += 1,
            _ => {}
        }
    }
    r
}

/// Whether a reply ends on two or more numbered lines that ask something.
fn ends_on_numbered_questions(text: &str) -> bool {
    let lines: Vec<&str> = text.trim_end().lines().rev().take_while(|l| !l.trim().is_empty()).collect();
    let numbered = lines.iter().filter(|l| {
        let l = l.trim_start();
        let digits = l.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && l[digits..].starts_with(['.', ')']) && (l.trim_end().ends_with('?') || l.trim_end().ends_with('？'))
    });
    numbered.count() >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(v: Value) -> String {
        v.to_string()
    }

    fn sample() -> String {
        [
            line(json!({ "type": "attachment", "sessionId": "s1", "cwd": "/w/frontend-t3", "gitBranch": "equipe/frontend-t3-ab12", "timestamp": "2026-11-06T01:00:00Z", "attachment": { "type": "skill_listing", "content": "- equipe:ui-floor: Use when…\n- equipe:browser: Use before…" } })),
            line(json!({ "type": "user", "message": { "content": "You're on T-3: add a Clear button." } })),
            line(json!({ "type": "assistant", "message": { "id": "m1", "content": [{ "type": "tool_use", "id": "t1", "name": "mcp__equipe__estimate", "input": { "done_in_minutes": 20 } }], "usage": { "input_tokens": 3, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000, "output_tokens": 10 } } })),
            line(json!({ "type": "assistant", "message": { "id": "m1", "content": [{ "type": "tool_use", "id": "t2", "name": "Bash", "input": { "command": "sleep 30 && ls" } }], "usage": { "input_tokens": 3, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000, "output_tokens": 12 } } })),
            line(json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t2", "is_error": true, "content": "Blocked: `sleep 30` waits in the foreground" }] } })),
            line(json!({ "type": "assistant", "message": { "id": "m2", "content": [{ "type": "tool_use", "id": "t3", "name": "Edit", "input": { "file_path": "/w/frontend-t3/src/App.tsx" } }], "usage": { "input_tokens": 1, "output_tokens": 5 } } })),
            line(json!({ "type": "assistant", "message": { "id": "m3", "content": [{ "type": "tool_use", "id": "t4", "name": "Skill", "input": { "skill": "equipe:ui-floor" } }] } })),
            line(json!({ "type": "assistant", "message": { "id": "m4", "content": [{ "type": "tool_use", "id": "t5", "name": "mcp__equipe__ask_decision", "input": { "questions": [{ "question": "Which?", "options": [{ "label": "A (Recommended)" }, { "label": "B" }] }] } }] } })),
            line(json!({ "type": "assistant", "message": { "id": "m5", "content": [{ "type": "text", "text": "Before I go on:\n\n1. Which colour?\n2. Which size?" }] } })),
            line(json!({ "type": "cost-state", "totalCostUSD": 0.5 })),
            line(json!({ "type": "cost-state", "totalCostUSD": 0.75, "timestamp": "2026-11-06T01:05:00Z" })),
            "not json".to_owned(),
        ]
        .join("\n")
    }

    #[test]
    fn a_transcript_reads_as_steps_with_each_message_counted_once() {
        let t = Transcript::parse(&sample());
        assert_eq!(t.session, "s1");
        assert_eq!(t.cwd.as_deref(), Some("/w/frontend-t3"));
        assert_eq!(t.role.as_deref(), Some("frontend"), "from the branch");
        assert_eq!(t.offered, [("ui-floor".to_owned(), 0), ("browser".to_owned(), 0)]);
        assert_eq!(t.tokens, Tokens { input: 4, cache_write: 100, cache_read: 1000, output: 17 }, "m1 once, at its last usage, and m2");
        assert_eq!(t.cost_usd, Some(0.75));
        assert_eq!(t.tools().map(|(n, _)| n).collect::<Vec<_>>(), ["mcp__equipe__estimate", "Bash", "Edit", "Skill", "mcp__equipe__ask_decision"]);
        assert!(matches!(&t.steps[3], Step::Answer { error: true, .. }));
        assert_eq!((t.first_at.as_deref(), t.last_at.as_deref()), (Some("2026-11-06T01:00:00Z"), Some("2026-11-06T01:05:00Z")));
    }

    #[test]
    fn a_skill_opened_right_after_its_situation_counts_and_never_is_never() {
        let t = Transcript::parse(&sample());
        let checks = skill_checks(&t, None);
        assert_eq!(checks, [SkillCheck { skill: "ui-floor".into(), trigger: "/w/frontend-t3/src/App.tsx".into(), opened: Opened::RightAfter }]);
        assert!(skill_checks(&t, Some("backend")).is_empty(), "the UI floor is for those who build screens");
        let read = Step::Tool { id: "x".into(), name: "Read".into(), input: json!({ "file_path": "/tmp/h/skills/1.0/x/skills/planning/SKILL.md" }) };
        assert!(opens(&read, "planning") && !opens(&read, "plan"));
        let plan = Transcript { offered: vec![("planning".into(), 0)], steps: vec![Step::Tool { id: "1".into(), name: "mcp__equipe__write_plan".into(), input: json!({}) }], ..Default::default() };
        assert_eq!(skill_checks(&plan, Some("pm"))[0].opened, Opened::Never);
        let designer = Transcript { offered: vec![("design-process".into(), 0)], steps: vec![Step::User("Design the empty state.".into()), Step::Tool { id: "1".into(), name: "Skill".into(), input: json!({ "skill": "equipe:design-process" }) }], ..Default::default() };
        assert_eq!(skill_checks(&designer, Some("designer"))[0].opened, Opened::RightAfter);
        let not_offered = Transcript { steps: plan.steps.clone(), ..Default::default() };
        assert!(skill_checks(&not_offered, Some("pm")).is_empty(), "a skill it wasn't offered isn't counted");
    }

    #[test]
    fn the_rules_count_what_the_steps_show() {
        let r = rules(&Transcript::parse(&sample()).steps);
        assert_eq!(r.tool_calls, 5);
        assert_eq!((r.foreground_sleeps, r.guard_refusals, r.estimates), (1, 1, 1));
        assert_eq!((r.decisions, r.decisions_with_options, r.decisions_recommended, r.recommended_in_label), (1, 1, 0, 1));
        assert_eq!(r.prose_questions, 1);
        let fine = [
            Step::Tool { id: "1".into(), name: "Bash".into(), input: json!({ "command": "sleep 2 && curl -s localhost:3000" }) },
            Step::Tool { id: "2".into(), name: "Bash".into(), input: json!({ "command": "sleep 60", "run_in_background": true }) },
            Step::Tool { id: "3".into(), name: "Bash".into(), input: json!({ "command": "echo 'equipe browser'" }) },
            Step::Tool { id: "4".into(), name: "Bash".into(), input: json!({ "command": "cd /tmp && /Users/x/.local/bin/equipe browser https://x.com" }) },
            Step::Tool { id: "5".into(), name: "mcp__equipe__ask_decision".into(), input: json!({ "questions": [{ "question": "Which?", "options": [{ "label": "A", "recommended": true }, { "label": "B" }] }] }) },
            Step::Tool { id: "6".into(), name: "AskUserQuestion".into(), input: json!({}) },
            Step::Reply("Done. Ask me if you want more.".into()),
        ];
        let r = rules(&fine);
        assert_eq!((r.foreground_sleeps, r.equipe_browser, r.decisions_recommended, r.recommended_in_label, r.engine_questions, r.prose_questions), (0, 1, 1, 0, 1, 0));
    }

    #[test]
    fn claude_codes_folder_for_a_directory_is_its_path_with_dashes() {
        assert_eq!(claude_project_dir_name(Path::new("/Users/abel/.equipe/worktrees/app/t428-6aebf592")), "-Users-abel--equipe-worktrees-app-t428-6aebf592");
        assert_eq!(board_tool("mcp__equipe__estimate"), "estimate");
        assert_eq!(role_in("/h/squads/acme/members/backend/memory/MEMORY.md").as_deref(), Some("backend"));
        assert_eq!(role_of_branch("equipe/qa-t12-x").as_deref(), Some("qa"));
        assert_eq!(role_of_branch("main"), None);
    }
}
