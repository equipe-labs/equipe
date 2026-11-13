//! Helpers: short-lived, read-only agents a member hands reading, surveying
//! and checking to — only a member whose engine has no subagents of its own.
//!
//! Claude Code has its Agent tool, and Codex on OpenAI its `multi_agent`; a
//! member on either keeps its engine's own and never sees these tools (two
//! ways to hand work off would leave the model to pick between them). Codex
//! on another provider is the gap: equipe turns `multi_agent` off there
//! ([`equipe_providers::codex_app::PLAIN_FEATURES_OFF`]), so such a member
//! gets `spawn_helper` and `helper_result` on equipe's board server instead.
//!
//! A helper runs on the member's own engine, provider and model, in the
//! member's working folder, in Codex's read-only sandbox with the network
//! off and no MCP server at all: it reads, searches and runs commands that
//! only look, and answers once. Its answer reaches the member word for word
//! — nothing summarises it on the way. At most [`AT_ONCE`] of a member's run
//! at once, the rest wait their turn; each has a time limit and a token
//! limit ([`Limits`]) and is stopped at either, the member getting what it
//! had and why. A member's turn ending, or the member stopping, stops its
//! helpers. The screens show each as one of the member's subagents.
//!
//! This module is the pure part: who gets the tools, their definitions, the
//! words, the queue, and what a helper's activity shows. The runtime runs
//! them ([`crate::runtime`]).

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use equipe_protocol::Engine;
use equipe_providers::turn::Activity;
use serde_json::{Value, json};

/// A member hands one narrow piece of reading or checking to a helper.
pub const SPAWN_HELPER: &str = "spawn_helper";
/// A member waits for its helpers' answers.
pub const HELPER_RESULT: &str = "helper_result";

/// How many of one member's helpers run at once; more wait their turn.
pub const AT_ONCE: usize = 3;

/// What bounds a helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long one helper may work, from when it starts (not while it waits its turn).
    pub time: Duration,
    /// How many tokens one helper's requests may process: input, cached input and output.
    pub tokens: u64,
    /// How long one `helper_result` waits for an answer before saying which still work.
    pub wait: Duration,
}

impl Limits {
    pub const DEFAULT: Limits = Limits { time: Duration::from_secs(10 * 60), tokens: 1_000_000, wait: Duration::from_secs(4 * 60) };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static LIMITS: Mutex<Limits> = Mutex::new(Limits::DEFAULT);

/// The limits helpers run under now.
pub fn limits() -> Limits {
    *LIMITS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Other limits, for this whole process: tests, and the fake daemon's screens.
#[doc(hidden)]
pub fn set_limits(limits: Limits) {
    *LIMITS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = limits;
}

/// How long Codex lets a call to equipe's board server run, for a member
/// with helpers: past the longest `helper_result` waits (its default is a minute).
pub fn tool_timeout_secs() -> u64 {
    limits().wait.as_secs() + 60
}

/// Whether a member gets helpers: only one whose engine runs tools
/// (`has_tools`: Claude Code, or Codex's CLI — not equipe's own Codex
/// sign-in, which only answers) but no subagents of its own, as
/// [`crate::roles::Subagents::of`] says: today Codex's CLI on a provider
/// other than OpenAI (`other_provider`), whichever it is.
pub fn offered(engine: Engine, has_tools: bool, other_provider: bool) -> bool {
    has_tools && crate::roles::Subagents::of(engine, has_tools, other_provider, None).is_none()
}

/// A member's tools on the board's server: its role's, and — when its
/// engine has no subagents of its own (`helped`) — the helpers'.
pub fn with_board(role: &str, helped: bool) -> Vec<Value> {
    let mut tools = crate::board::tools_for(role);
    if helped {
        tools.extend(self::tools());
    }
    tools
}

/// The tools, as MCP tool definitions.
pub fn tools() -> Vec<Value> {
    let Limits { time, wait, .. } = limits();
    vec![
        json!({
            "name": SPAWN_HELPER,
            "description": format!("Hand one narrow piece of reading, surveying or checking to a helper: a short-lived agent \
on your own model, in your working folder, that reads files, searches and runs commands that only look — it never edits, \
commits or sends anything. It answers you once, concisely, with path:line evidence and what it couldn't check. Returns at \
once with the helper's id: up to {AT_ONCE} of yours run at once and more wait their turn; each works at most {} minutes. \
Collect every answer with `{HELPER_RESULT}` before you end your turn — your helpers stop when it ends.", time.as_secs() / 60),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brief": {
                        "type": "string",
                        "description": "All it needs, since it knows nothing of your conversation: the question, where to \
look, what you already ruled out, and what to hand back.",
                    },
                    "what": {
                        "type": "string",
                        "description": "A few plain words for the person, who sees it at work under your name: \"find where sessions expire\".",
                    },
                },
                "required": ["brief"],
            },
        }),
        json!({
            "name": HELPER_RESULT,
            "description": format!("Your helpers' answers: the one named, or without an id the next to finish — waiting up \
to {} minutes, then saying which still work. Each answer comes word for word, with how it ended.", wait.as_secs().div_ceil(60)),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A helper's id, as spawn_helper gave it: h1. Left out: whichever finishes next." },
                },
            },
        }),
    ]
}

/// What a member whose engine has no subagents is told about helpers, in
/// place of the words for no subagents ([`crate::roles`]): reading and
/// checking to helpers, big changes to teammates through tickets — the PM
/// putting them on the board (`pm`), any other proposing them.
pub fn charter_paragraph(pm: bool) -> String {
    let tickets = if pm {
        "put each on a ticket of its own with `create_ticket`, for teammates who work at once"
    } else {
        "propose each as a ticket of its own with `create_ticket`, for the PM to hand to teammates who work at once"
    };
    format!(
        "Your engine has no subagents of its own; equipe gives you helpers instead (`{SPAWN_HELPER}`, `{HELPER_RESULT}`). \
A helper is a short-lived agent on your own model, in your working folder, that only reads: it searches, reads files and \
runs commands that only look, and answers you once — concisely, with path:line evidence and what it couldn't check. Hand \
reading, surveying and checking to helpers — learning unfamiliar code, reading logs, finding every caller of something, \
checking a change against the tests or the spec — several at once when the parts don't depend on each other (up to \
{AT_ONCE} run at once; more wait their turn), each with a brief that stands on its own: the question, where to look, what \
you ruled out, what to hand back. Keep every edit yourself: helpers never change files, and you put their answers together \
and check them before you rely on them. While they work, go on with what doesn't need their answers, then collect every \
answer with `{HELPER_RESULT}` before you end your turn — helpers stop when it ends. Look yourself at what is quicker to read \
than to describe. The person sees each helper at work under your name, so give each a few plain words (`what`). \
Working in parallel is the squad's strength: anything that would take you alone more than about 20 minutes and changes \
many files — a big ticket, a broad change — you split into parts that don't depend on each other and {tickets}, each told \
exactly its part; you work through your own part in order."
    )
}

/// A helper's own instructions: who it works for, what it may do, how to answer.
pub fn instructions(member: &str, cwd: &Path, limits: &Limits) -> String {
    format!(
        "You are a helper for {member}, a member of a squad that equipe runs for one person. {member} handed you one \
narrow job: do just that, then answer. You work read-only in {member}'s working folder, {cwd}: read files, search, list, \
and run commands that only look (`git log`, `git diff`, `git grep`, `rg`, a test run that writes nothing). You can't change \
anything, and don't try: no edits, no new files, no git commits, branches or pushes, nothing sent or posted anywhere — the \
sandbox refuses writes and the network, and {member} makes every change itself. Your answer goes to {member} word for \
word, read by no one else first, so make it what {member} needs: the finding first, concisely; each claim with its \
evidence as path:line, or the command and what it printed; and plainly what you couldn't check, and why. You have at most \
{} minutes: answer well before, with what you have.",
        limits.time.as_secs() / 60,
        cwd = cwd.display(),
    )
}

/// The few words a helper shows as: what the member called it, else its brief's first line.
pub fn what_of(what: Option<&str>, brief: &str) -> String {
    let words = what.map(str::trim).filter(|w| !w.is_empty()).unwrap_or_else(|| brief.trim().lines().next().unwrap_or("").trim());
    words.chars().take(80).collect()
}

/// A helper's turn: read-only, its own instructions, the member's model on
/// the member's provider, and no MCP server — not equipe's, not the person's.
pub fn request<'a>(
    brief: &'a str,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    instructions: &'a str,
    env: &'a [(String, String)],
    provider: &'a equipe_providers::codex_app::Provider,
) -> equipe_providers::codex_app::TurnRequest<'a> {
    equipe_providers::codex_app::TurnRequest {
        prompt: brief,
        model,
        effort,
        access: equipe_providers::claude_code::Access::ReadOnly,
        resume: None,
        fork: false,
        fresh_prompt: None,
        web_search: false,
        instructions: Some(instructions),
        mcp_servers: Some(&NO_SERVERS),
        writable: &[],
        env,
        provider: Some(provider),
        // None of its own on another provider, nor any to cap.
        subagents: None,
        compact_at: None,
    }
}

/// No MCP server at all: given none of its own, a thread has the person's switched off.
static NO_SERVERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({}));

// ── the queue ──

/// Whether a helper starts now, or waits behind `ahead` others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Now,
    Queued { ahead: usize },
}

/// One member's helpers by id: running, waiting their turn, and answered
/// but not yet collected.
#[derive(Debug, Default)]
pub struct Pool {
    next: u32,
    running: Vec<String>,
    queued: VecDeque<String>,
    answered: Vec<Answer>,
}

impl Pool {
    /// A new helper: its id, and whether it starts now.
    pub fn spawn(&mut self) -> (String, Start) {
        self.next += 1;
        let id = format!("h{}", self.next);
        if self.running.len() < AT_ONCE {
            self.running.push(id.clone());
            (id, Start::Now)
        } else {
            self.queued.push_back(id.clone());
            (id, Start::Queued { ahead: self.queued.len() - 1 })
        }
    }

    /// A running helper ended with `answer`, kept to be collected; the
    /// helper waiting longest starts in its place, if one does.
    pub fn ended(&mut self, answer: Answer) -> Option<String> {
        let at = self.running.iter().position(|r| *r == answer.id)?;
        self.running.remove(at);
        self.answered.push(answer);
        let next = self.queued.pop_front()?;
        self.running.push(next.clone());
        Some(next)
    }

    /// Everything stops: the running ones' ids, to stop. Those waiting never
    /// start, and answers not collected go.
    pub fn stop_all(&mut self) -> Vec<String> {
        self.queued.clear();
        self.answered.clear();
        std::mem::take(&mut self.running)
    }

    /// The answers in, taken: `id`'s, or every one.
    pub fn take(&mut self, id: Option<&str>) -> Vec<Answer> {
        let (taken, kept) = std::mem::take(&mut self.answered).into_iter().partition(|a| id.is_none_or(|id| a.id == id));
        self.answered = kept;
        taken
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.iter().any(|r| r == id)
    }

    pub fn is_queued(&self, id: &str) -> bool {
        self.queued.iter().any(|q| q == id)
    }

    /// Whether `id` is one of these that hasn't been collected.
    pub fn knows(&self, id: &str) -> bool {
        self.is_running(id) || self.is_queued(id) || self.answered.iter().any(|a| a.id == id)
    }

    /// Running and waiting, in that order.
    pub fn outstanding(&self) -> impl Iterator<Item = &String> {
        self.running.iter().chain(self.queued.iter())
    }

    pub fn has_answers(&self) -> bool {
        !self.answered.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty() && self.queued.is_empty() && self.answered.is_empty()
    }
}

// ── answers ──

/// How a helper ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Answered,
    /// Stopped at its time limit.
    TimeLimit,
    /// Stopped at its token limit.
    TokenLimit,
    /// The person stopped it.
    Stopped,
    /// Its engine failed: why.
    Failed(String),
}

/// A helper's answer, as the member collects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub id: String,
    pub what: String,
    pub ended: Ended,
    /// What it wrote — its answer, or as far as it got — untouched.
    pub text: String,
    pub took: Duration,
    /// The steps it took, oldest first: what a stopped one covered.
    pub steps: Vec<String>,
}

/// Most of a stopped helper's steps the member is told.
const STEPS_TOLD: usize = 12;

/// An answer for the member: a line on how it ended, then the helper's own
/// words exactly as it wrote them.
pub fn answer_text(answer: &Answer, limits: &Limits) -> String {
    let Answer { id, what, ended, text, took, steps } = answer;
    let took = span(*took);
    let head = match ended {
        Ended::Answered => format!("Helper {id} ({what}) answered after {took}:"),
        Ended::TimeLimit => format!("Helper {id} ({what}) was stopped at its time limit, {} minutes. What it had by then:", limits.time.as_secs() / 60),
        Ended::TokenLimit => format!("Helper {id} ({what}) was stopped at its token limit, {} tokens, after {took}. What it had by then:", limits.tokens),
        Ended::Stopped => format!("Helper {id} ({what}) was stopped by the person after {took}. What it had by then:"),
        Ended::Failed(why) => format!("Helper {id} ({what}) failed after {took}: {why}. What it had by then:"),
    };
    let body = if text.trim().is_empty() { "(nothing written yet)".to_owned() } else { text.clone() };
    if *ended == Ended::Answered {
        return format!("{head}\n\n{body}");
    }
    let mut out = format!("{head}\n\n{body}");
    if !steps.is_empty() {
        let skipped = steps.len().saturating_sub(STEPS_TOLD);
        let shown: Vec<&str> = steps.iter().skip(skipped).map(String::as_str).collect();
        let earlier = if skipped > 0 { format!("{skipped} earlier, then ") } else { String::new() };
        out.push_str(&format!("\n\nSteps it took: {earlier}{}.", shown.join("; ")));
    }
    out.push_str("\n\nWhat it didn't get to is unchecked: check it yourself, or hand it to a new helper with a narrower brief.");
    out
}

/// What `spawn_helper` answers.
pub fn spawned_text(id: &str, start: Start) -> String {
    match start {
        Start::Now => format!("Helper {id} is at work. Go on with what doesn't need its answer; collect it with `{HELPER_RESULT}`."),
        Start::Queued { ahead } => format!(
            "Helper {id} waits its turn: {AT_ONCE} of yours are at work{}. It starts when one finishes; collect it with `{HELPER_RESULT}`.",
            if ahead > 0 { format!(" and {ahead} more wait before it") } else { String::new() }
        ),
    }
}

/// The helpers still out, for the end of a `helper_result`: each running one
/// with how long it has worked, then those waiting.
pub fn still_out(running: &[(String, String, Duration)], waiting: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    if !running.is_empty() {
        let each: Vec<String> = running.iter().map(|(id, what, at)| format!("{id} ({what}), {}", span(*at))).collect();
        parts.push(format!("Still at work: {}.", each.join("; ")));
    }
    if !waiting.is_empty() {
        let each: Vec<String> = waiting.iter().map(|(id, what)| format!("{id} ({what})")).collect();
        parts.push(format!("Waiting their turn: {}.", each.join("; ")));
    }
    parts.join(" ")
}

/// `2m 14s`, `40s`, `1h 03m`.
pub fn span(took: Duration) -> String {
    let secs = took.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

// ── what a helper shows ──

/// A helper's activity, as its member's subagent's: each step it takes, each
/// step done, and what it says between them. Its tokens, kept.
#[derive(Debug, Default)]
pub struct Shown {
    /// What it said since its last step: its words between steps — or, at
    /// the end, its answer, which goes as the result instead.
    said: String,
    /// What it said before, between its steps.
    words: Vec<String>,
    pub steps: Vec<String>,
    pub spent: u64,
}

impl Shown {
    /// Everything it has said so far, in order: what a helper stopped
    /// before its answer had.
    pub fn so_far(&self) -> String {
        let mut all = self.words.clone();
        if !self.said.trim().is_empty() {
            all.push(self.said.trim().to_owned());
        }
        all.join("\n\n")
    }

    /// The member's subagent activities for helper `id`'s `activity`.
    pub fn on(&mut self, id: &str, activity: Activity) -> Vec<Activity> {
        match activity {
            Activity::Text(text) => {
                self.said.push_str(&text);
                Vec::new()
            }
            Activity::Tool { name, target, .. } => {
                let step = format!("{name} {}", target.unwrap_or_default()).trim().to_owned();
                self.steps.push(step.clone());
                let mut out = Vec::new();
                let said = std::mem::take(&mut self.said);
                if !said.trim().is_empty() {
                    self.words.push(said.trim().to_owned());
                    out.push(Activity::SubagentText { id: id.to_owned(), text: said.trim().to_owned() });
                }
                out.push(Activity::SubagentStep { id: id.to_owned(), step });
                out
            }
            Activity::ToolDone(done) => vec![Activity::SubagentTool { id: id.to_owned(), done }],
            Activity::Spent(tokens) => {
                self.spent = tokens;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_providers::turn::ToolDone;

    #[test]
    fn only_codex_on_another_provider_gets_helpers() {
        assert!(offered(Engine::Codex, true, true), "Codex's CLI on Nebius: no subagents of its own");
        assert!(!offered(Engine::Codex, true, false), "Codex on OpenAI keeps its multi_agent");
        assert!(!offered(Engine::Claude, true, false) && !offered(Engine::Claude, true, true), "Claude Code keeps its Agent tool");
        assert!(!offered(Engine::Codex, false, true), "equipe's own Codex sign-in runs no tools");
    }

    #[test]
    fn the_helpers_tools_come_only_with_the_word_for_them() {
        let names = |tools: Vec<Value>| tools.iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        for role in ["pm", "backend", "designer"] {
            let plain = names(with_board(role, false));
            assert_eq!(plain, names(crate::board::tools_for(role)), "{role}: as before");
            assert!(!plain.iter().any(|n| n == SPAWN_HELPER || n == HELPER_RESULT), "{role}");
            let helped = names(with_board(role, true));
            assert_eq!(helped[..plain.len()], plain[..], "{role}: its own tools first, as they were");
            assert_eq!(helped[plain.len()..], [SPAWN_HELPER.to_owned(), HELPER_RESULT.to_owned()], "{role}");
        }
    }

    #[test]
    fn the_tools_say_what_a_helper_may_do_and_ask_for_a_brief() {
        let tools = tools();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [SPAWN_HELPER, HELPER_RESULT]);
        let spawn = &tools[0];
        assert_eq!(spawn["inputSchema"]["required"], json!(["brief"]));
        let doc = spawn["description"].as_str().unwrap();
        for part in ["never edits, commits or sends anything", "path:line", "up to 3", "10 minutes", "your helpers stop when it ends"] {
            assert!(doc.contains(part), "{part}: {doc}");
        }
        assert!(tools[1]["inputSchema"]["properties"]["id"].is_object());
    }

    #[test]
    fn three_run_at_once_and_the_rest_wait_their_turn() {
        let mut pool = Pool::default();
        let started: Vec<(String, Start)> = (0..5).map(|_| pool.spawn()).collect();
        assert_eq!(started[..3], [("h1".into(), Start::Now), ("h2".into(), Start::Now), ("h3".into(), Start::Now)]);
        assert_eq!(started[3], ("h4".into(), Start::Queued { ahead: 0 }));
        assert_eq!(started[4], ("h5".into(), Start::Queued { ahead: 1 }));
        assert!(pool.is_running("h3") && pool.is_queued("h4"));
        // One answers: the one waiting longest starts in its place, never more than three.
        let next = pool.ended(answer("h2", Ended::Answered, "found it"));
        assert_eq!(next.as_deref(), Some("h4"));
        assert_eq!(pool.outstanding().filter(|id| pool.is_running(id)).count(), AT_ONCE);
        assert_eq!(pool.ended(answer("h9", Ended::Answered, "")), None, "one it doesn't run changes nothing");
        // Its answer waits to be collected, by id or with the others.
        assert!(pool.has_answers() && pool.knows("h2"));
        assert!(pool.take(Some("h1")).is_empty());
        assert_eq!(pool.take(None).iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["h2"]);
        assert!(!pool.knows("h2"), "collected once");
    }

    #[test]
    fn stopping_stops_the_running_and_starts_none_waiting() {
        let mut pool = Pool::default();
        for _ in 0..4 {
            pool.spawn();
        }
        pool.ended(answer("h1", Ended::Answered, "x"));
        let stopped = pool.stop_all();
        assert_eq!(stopped, ["h2", "h3", "h4"]);
        assert!(pool.is_empty(), "nothing waits, nothing to collect");
        assert_eq!(pool.ended(answer("h2", Ended::Stopped, "")), None);
    }

    #[test]
    fn an_answer_comes_word_for_word() {
        let text = "Sessions expire in `src/auth.rs:42` (`TTL = 3600`).\n\n- Not checked: the mobile client.\n\n```rust\nlet x = 1;\n```";
        let said = answer_text(&answer("h1", Ended::Answered, text), &Limits::DEFAULT);
        assert_eq!(said, format!("Helper h1 (find where sessions expire) answered after 2m 14s:\n\n{text}"));
        assert!(said.ends_with(text), "nothing summarised, nothing added after it");
    }

    #[test]
    fn a_stopped_helper_hands_in_what_it_had_and_why() {
        let limits = Limits::DEFAULT;
        let mut timed = answer("h2", Ended::TimeLimit, "So far: `a.rs:3` reads the TTL.");
        timed.steps = (1..=14).map(|n| format!("Read f{n}.rs")).collect();
        let said = answer_text(&timed, &limits);
        assert!(said.starts_with("Helper h2 (find where sessions expire) was stopped at its time limit, 10 minutes. What it had by then:\n\nSo far: `a.rs:3` reads the TTL."), "{said}");
        assert!(said.contains("Steps it took: 2 earlier, then Read f3.rs; ") && said.contains("Read f14.rs."), "{said}");
        assert!(said.contains("unchecked"), "{said}");
        let spent = answer_text(&answer("h3", Ended::TokenLimit, ""), &limits);
        assert!(spent.contains("its token limit, 1000000 tokens") && spent.contains("(nothing written yet)"), "{spent}");
        assert!(answer_text(&answer("h4", Ended::Stopped, "x"), &limits).contains("stopped by the person"));
        assert!(answer_text(&answer("h5", Ended::Failed("the provider refused the key".into()), "x"), &limits).contains("failed after 2m 14s: the provider refused the key"));
    }

    #[test]
    fn its_instructions_keep_it_reading_and_its_answer_for_the_member() {
        let text = instructions("frontend", Path::new("/w/acme"), &Limits::DEFAULT);
        for part in ["You are a helper for frontend", "read-only in frontend's working folder, /w/acme", "no edits, no new files, no git commits, branches or pushes, nothing sent or posted anywhere", "path:line", "what you couldn't check", "at most 10 minutes", "word for word"] {
            assert!(text.contains(part), "{part}: {text}");
        }
    }

    #[test]
    fn what_a_helper_does_shows_as_a_subagents_steps() {
        let mut shown = Shown::default();
        assert!(shown.on("h1", Activity::Text("Looking at ".into())).is_empty());
        assert!(shown.on("h1", Activity::Text("auth.".into())).is_empty());
        let step = shown.on("h1", Activity::Tool { name: "Read".into(), target: Some("src/auth.rs".into()), label: None, background: false });
        assert_eq!(step, vec![Activity::SubagentText { id: "h1".into(), text: "Looking at auth.".into() }, Activity::SubagentStep { id: "h1".into(), step: "Read src/auth.rs".into() }]);
        let done = ToolDone { name: "Read".into(), target: Some("src/auth.rs".into()), ok: true, exit: None, ms: 3, output: String::new(), diff: None };
        assert_eq!(shown.on("h1", Activity::ToolDone(done.clone())), vec![Activity::SubagentTool { id: "h1".into(), done }]);
        assert!(shown.on("h1", Activity::Spent(4200)).is_empty());
        assert_eq!((shown.spent, shown.steps.clone()), (4200, vec!["Read src/auth.rs".to_owned()]));
        assert!(shown.on("h1", Activity::Text("The answer".into())).is_empty(), "the answer goes as its result, not as words between steps");
        assert_eq!(shown.so_far(), "Looking at auth.\n\nThe answer", "all it said, for one stopped before it answered");
    }

    #[test]
    fn spans_read_short() {
        assert_eq!((span(Duration::from_secs(40)), span(Duration::from_secs(134)), span(Duration::from_secs(3780))), ("40s".into(), "2m 14s".into(), "1h 03m".into()));
        assert_eq!(what_of(None, "Find where sessions expire\nmore"), "Find where sessions expire");
        assert_eq!(what_of(Some("  check the docs "), "x"), "check the docs");
    }

    fn answer(id: &str, ended: Ended, text: &str) -> Answer {
        Answer { id: id.into(), what: "find where sessions expire".into(), ended, text: text.into(), took: Duration::from_secs(134), steps: Vec::new() }
    }
}
