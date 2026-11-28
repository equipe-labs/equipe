//! Independent review before a builder's work lands — the rules, with no
//! runtime in them (the runtime's side is `runtime::review_gate`).
//!
//! One engineer builds a change end to end; reviewers who don't write code
//! check it before it lands, each in a fresh run of its own that never saw
//! it being built:
//!
//! - **The security reviewer** reads every change that touches a server or
//!   data — validation of every write, rate limits, sign-in and ownership,
//!   secrets — and runs the project's static checks and a scan.
//! - **QA** drives the real app against the ticket's acceptance criteria
//!   (agreed when it was planned), skeptically, with pictures as evidence;
//!   where the squad has the platform, `qa_check` must pass.
//!
//! A reviewer that finds a real defect sends the work back to its builder
//! with the findings; one that passes it says what it ran and what it saw.
//! "Done" is never the builder's own word: a pass without evidence — what
//! was run, pictures of a screen it checked, a passing `qa_check` — is
//! refused ([`read`]). Reviews go at most [`ROUNDS`] rounds a change: after
//! that it goes on to the PM with what is still open, rather than round and
//! round (a reviewer asked to find problems always finds one).
//!
//! The research behind it: docs/research and the 2026-11-28 architecture
//! report — one writer per coupled change, reviewers in a fresh context,
//! done decided by tools and evidence.

use equipe_protocol::TicketKind;
use serde_json::{Value, json};

use crate::board;

/// The reviewer's tool: its verdict on the change it reviewed.
pub const REVIEW_VERDICT: &str = "review_verdict";

/// How many rounds of review a change goes through at most.
pub const ROUNDS: u32 = 2;

/// Who reviews a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reviewer {
    /// The backend and security reviewer: server and data changes.
    Security,
    /// The QA evaluator: the app driven against what it must do.
    Qa,
}

impl Reviewer {
    pub const ALL: [Reviewer; 2] = [Reviewer::Security, Reviewer::Qa];

    /// The role that reviews.
    pub fn role(self) -> &'static str {
        match self {
            Reviewer::Security => crate::roles::SECURITY,
            Reviewer::Qa => "qa",
        }
    }

    /// How it is kept.
    pub fn code(self) -> &'static str {
        self.role()
    }

    pub fn parse(code: &str) -> Option<Reviewer> {
        Self::ALL.into_iter().find(|r| r.code() == code)
    }

    /// What it is, in a few English words: for the PM and the record.
    pub fn what(self) -> &'static str {
        match self {
            Reviewer::Security => "security review",
            Reviewer::Qa => "QA",
        }
    }
}

/// Whether `path`, changed, is server or data code: an API, a worker, a
/// function, a database, its schema and migrations, sign-in, or the
/// configuration a server runs with.
pub fn server_or_data_file(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    let dirs = [
        "api/", "server/", "servers/", "backend/", "worker/", "workers/", "functions/", "netlify/functions/", "supabase/", "prisma/",
        "migrations/", "migration/", "db/", "database/", "schema/", "auth/", "middleware/", "routes/api/", "app/api/", "pages/api/",
        "src/server/", "src/api/", "src/db/", "src/lib/server/", "edge/", "lambda/",
    ];
    let segment = |d: &str| p.starts_with(d) || p.contains(&format!("/{d}"));
    let names = [
        "wrangler.toml", "wrangler.json", "wrangler.jsonc", "schema.prisma", "schema.sql", "firestore.rules", "storage.rules", "middleware.ts",
        "middleware.js", "server.js", "server.ts", "server.mjs", "worker.js", "worker.ts", "_worker.js", "hooks.server.ts", "hooks.server.js",
    ];
    dirs.iter().any(|d| segment(d))
        || names.contains(&name)
        || name.ends_with(".sql")
        || name.contains("+server.")
        || name.contains(".server.")
        || name == "route.ts"
        || name == "route.js"
        || name.starts_with("actions.")
}

/// Whether `path` is part of the product people use — anything but notes,
/// docs, tests and the repository's own housekeeping.
pub fn product_file(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    let aside = p.starts_with("docs/")
        || p.starts_with("design/")
        || p.starts_with(".github/")
        || p.starts_with(".equipe/")
        || p.contains("/__tests__/")
        || p.starts_with("test/")
        || p.starts_with("tests/")
        || p.starts_with("e2e/")
        || name.ends_with(".md")
        || name.contains(".test.")
        || name.contains(".spec.")
        || matches!(name, "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "bun.lockb" | ".gitignore" | "license");
    !aside
}

/// Whether `path` changes how a screen looks or behaves.
pub fn screen_file(path: &str) -> bool {
    [".tsx", ".jsx", ".css", ".scss", ".html", ".vue", ".svelte", ".astro", ".swift", ".kt"].iter().any(|x| path.to_ascii_lowercase().ends_with(x))
}

/// The reviews a change needs before it lands: a builder's code change, by
/// what its branch changed and the reviewers the squad has. The security
/// reviewer on anything touching a server or data; QA on anything of the
/// product people use. A squad without the reviewer goes on without it.
pub fn needed(builder_role: &str, kind: TicketKind, changed: &[String], has_reviewer: impl Fn(Reviewer) -> bool) -> Vec<Reviewer> {
    if kind != TicketKind::Code || !crate::roles::builds(builder_role) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if has_reviewer(Reviewer::Security) && changed.iter().any(|f| server_or_data_file(f)) {
        out.push(Reviewer::Security);
    }
    if has_reviewer(Reviewer::Qa) && changed.iter().any(|f| product_file(f)) {
        out.push(Reviewer::Qa);
    }
    out
}

/// What QA's run saw of the platform's `qa_check`: the last call's word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QaSeen {
    /// Every check passed (review findings may stand beside it).
    Passed,
    /// These checks failed: key and what is wrong.
    Failed(Vec<String>),
    /// It could not run: the app has no deployed copy yet, or the tool failed.
    Unavailable(String),
}

impl QaSeen {
    pub fn failed(&self) -> bool {
        matches!(self, QaSeen::Failed(_))
    }
}

/// Whether a tool the engine reports is the platform's `qa_check`.
pub fn is_qa_check(tool: &str) -> bool {
    tool == "qa_check" || tool.ends_with("__qa_check")
}

/// Whether a tool the engine reports is the platform's `deploy_site`.
pub fn is_deploy(tool: &str) -> bool {
    tool == "deploy_site" || tool.ends_with("__deploy_site")
}

/// What a `qa_check` answer says: `{ ok, url, checks: [{ key, ok, review?, detail }] }`.
/// A call that errored, or an answer that can't be read, is [`QaSeen::Unavailable`].
pub fn read_qa_check(ok_call: bool, output: &str) -> QaSeen {
    let start = output.find('{');
    let parsed: Option<Value> = start.and_then(|s| serde_json::from_str(output[s..].trim()).ok().or_else(|| {
        let end = output.rfind('}')?;
        serde_json::from_str(&output[s..=end]).ok()
    }));
    let Some(v) = parsed.filter(|v| v.get("ok").is_some()) else {
        let first = output.lines().find(|l| !l.trim().is_empty()).unwrap_or("no answer").trim();
        return QaSeen::Unavailable(first.chars().take(200).collect());
    };
    if !ok_call {
        return QaSeen::Unavailable(v.get("error").and_then(Value::as_str).unwrap_or("the check could not run").chars().take(200).collect());
    }
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        return QaSeen::Passed;
    }
    let failed: Vec<String> = v
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("ok").and_then(Value::as_bool) == Some(false) && c.get("review").and_then(Value::as_bool) != Some(true))
        .map(|c| {
            let key = c.get("key").and_then(Value::as_str).unwrap_or("check");
            let detail = c.get("detail").and_then(Value::as_str).unwrap_or("");
            if detail.is_empty() { key.to_owned() } else { format!("{key}: {}", detail.chars().take(160).collect::<String>()) }
        })
        .collect();
    QaSeen::Failed(if failed.is_empty() { vec!["qa_check: not ok".to_owned()] } else { failed })
}

/// Why a builder's hand-in can't go in yet for want of evidence: it put the
/// app on the platform in this work, and the platform's checklist hasn't
/// passed on it since. `None`: nothing holds it.
pub fn hand_in_refused(deployed: bool, qa: Option<&QaSeen>) -> Option<String> {
    if !deployed {
        return None;
    }
    match qa {
        Some(QaSeen::Passed) | Some(QaSeen::Unavailable(_)) => None,
        Some(QaSeen::Failed(failed)) => Some(format!(
            "Not handed in: the app's last `qa_check` failed — {}. Fix each, deploy again and run `qa_check` again; the work is \
done only when it passes.",
            failed.join("; ")
        )),
        None => Some(
            "Not handed in: you deployed this work, and \"done\" needs evidence from the deployed copy. Run `qa_check` on the app \
(with `rename_to`/`rename_from` when this renames it), fix what fails, then hand in again."
                .to_owned(),
        ),
    }
}

/// The reviewer's `review_verdict`, as an MCP tool definition.
pub fn verdict_tool() -> Value {
    json!({
        "name": REVIEW_VERDICT,
        "description": "Finish the review you are on: your verdict on the change, with your evidence. `pass` only when every \
acceptance criterion and check you ran held; `changes` when you found a real defect — it goes back to its engineer with your \
findings. You never fix it yourself. Findings are the defects only, the worst first, each with where it is, how to reproduce \
it and the fix in a sentence; leave style preferences out. `ran` lists what you actually ran or did — commands, checks, the \
flows you clicked through — and a QA pass of a change people see carries your own pictures of it in `images`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "enum": ["pass", "changes"] },
                "findings": { "type": "string", "description": "The defects found, worst first; for a pass, a line or two on what holds." },
                "ran": { "type": "array", "items": { "type": "string" }, "description": "What you ran or did, one item each: `npm run lint` passed, `semgrep --config auto` 0 findings, booking flow at 390px, …" },
                "criteria": { "type": "array", "items": { "type": "string" }, "description": "QA: each acceptance criterion with pass or fail and what you saw." },
                "images": { "type": "array", "items": { "type": "string" }, "description": "Pictures you took of what you checked: paths in your folder." },
            },
            "required": ["verdict", "findings", "ran"],
        },
    })
}

/// A reviewer's verdict, read and judged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub passed: bool,
    /// The findings, then what it ran and the criteria it checked.
    pub findings: String,
}

/// What a `review_verdict` call says, refused when its evidence is missing:
/// a verdict and findings, what was run; for QA's pass of a change people
/// see, pictures of its own; where the squad has the platform, a `qa_check`
/// that passed (or could not run) in QA's run.
pub fn read(args: &Value, reviewer: Reviewer, own_pictures: usize, screens: bool, platform: bool, qa: Option<&QaSeen>) -> Result<Verdict, String> {
    let verdict = board::arg_str(args, "verdict").map(|v| v.trim().to_ascii_lowercase());
    let passed = match verdict.as_deref() {
        Some("pass" | "passed") => true,
        Some("changes" | "changes_requested" | "fail" | "failed") => false,
        _ => return Err("Give `verdict`: `pass` or `changes`.".into()),
    };
    let findings = board::arg_str(args, "findings").map(str::trim).filter(|f| !f.is_empty()).ok_or("Give `findings`: the defects found, worst first (for a pass, a line on what holds).")?;
    let ran: Vec<String> = board::arg_strs(args, "ran").into_iter().map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()).collect();
    if ran.is_empty() {
        return Err("Give `ran`: what you actually ran or did — the commands and checks with their result, the flows you clicked through. A verdict without it is only a claim.".into());
    }
    let criteria: Vec<String> = board::arg_strs(args, "criteria").into_iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
    if passed && reviewer == Reviewer::Qa {
        if criteria.is_empty() {
            return Err("A QA pass names each acceptance criterion with pass or fail and what you saw, in `criteria`.".into());
        }
        if screens && own_pictures == 0 {
            return Err("A QA pass of a change people see carries your own pictures of it, in `images`: take them as you click through, then finish again.".into());
        }
        if platform {
            match qa {
                Some(QaSeen::Failed(failed)) => {
                    return Err(format!("Not a pass: `qa_check` failed in your run — {}. Send it back with `changes` and these as findings.", failed.join("; ")));
                }
                None => return Err("Run `qa_check` on the app first (with `rename_to`/`rename_from` when the work renames it): a pass needs it to pass.".into()),
                Some(QaSeen::Passed | QaSeen::Unavailable(_)) => {}
            }
        }
    }
    let mut text = findings.to_owned();
    text.push_str(&format!("\n\nRan: {}", ran.join("; ")));
    if !criteria.is_empty() {
        text.push_str(&format!("\nCriteria: {}", criteria.join("; ")));
    }
    if let Some(QaSeen::Unavailable(why)) = qa.filter(|_| reviewer == Reviewer::Qa && platform) {
        text.push_str(&format!("\nqa_check couldn't run: {why}"));
    }
    Ok(Verdict { passed, findings: text })
}

/// What the reviewer is given to do: the change, where it is, what it must
/// do, and how to finish — in English, for the member.
/// The change a review is of: its ticket, who built it, and where its work is.
pub struct Change<'a> {
    pub ticket: crate::protocol::TicketId,
    pub title: &'a str,
    pub body: &'a str,
    pub builder: &'a str,
    pub worktree: Option<&'a str>,
    pub branch: Option<&'a str>,
}

pub fn brief(reviewer: Reviewer, change: &Change<'_>, round: u32, platform: bool) -> String {
    let Change { ticket, title, body, builder, worktree, branch } = *change;
    let place = match (worktree, branch) {
        (Some(w), Some(b)) => format!("Its work is on branch `{b}`, checked out at `{w}`: read the diff there (`git -C {w} diff main...HEAD`) and the code around it."),
        (Some(w), None) => format!("Its work is checked out at `{w}`: read the diff there (`git -C {w} diff main...HEAD`)."),
        (None, Some(b)) => format!("Its work is on branch `{b}` of the project: read its diff against main."),
        (None, None) => "Read its diff against main in the project.".to_owned(),
    };
    let again = if round > 1 { format!(" This is round {round}: its engineer fixed what the last review found — check those first, then the rest.") } else { String::new() };
    let how = match reviewer {
        Reviewer::Security => "Review it as the squad's backend and security reviewer, from scratch — not from its engineer's account of it. Check \
every write the change accepts is validated on the server (lengths, formats, ranges, dates that may not be in the past), writes are \
rate-limited, sign-in and ownership are checked where they are needed, no visitor can read or change another's data, secrets stay in \
the server's environment, and errors leak nothing. Run the project's own linters and type check, and a security scan where one is \
installed (`semgrep --config auto`, `npm audit --omit=dev`); read their findings."
            .to_owned(),
        Reviewer::Qa => {
            let qa_check = if platform {
                " Run the platform's `qa_check` on the app (with `rename_to`/`rename_from` when the work renames it) and carry its \
result: a failed check is a failure."
            } else {
                ""
            };
            format!(
                "Evaluate it as the squad's QA, skeptically: drive the real app in a browser — its preview link in the hand-in, or \
the build run from its worktree — and check each acceptance criterion below one by one, with good, empty, too long and wrong input, at \
phone and desktop width. Each is pass or fail with what you saw and a picture; one you couldn't run is said so, never passed.{qa_check}"
            )
        }
    };
    format!(
        "Review T-{ticket} \"{title}\", which {builder} built and handed in.{again} {place}\n\n{how}\n\nYou don't change any file. Finish \
with `{REVIEW_VERDICT}`: `pass` with what you ran, or `changes` with the defects — only real ones against what the work must do, \
worst first, each with where and how to fix it; no style preferences. Its engineer fixes them.\n\nWhat it must do (its ticket):\n{body}"
    )
}

/// The note its builder reads when a review sends the work back.
pub fn sent_back_note(reviewer: Reviewer, by: &str, findings: &str) -> String {
    format!("Sent back by {by}'s {}: fix each finding, then hand in again.\n{findings}", reviewer.what())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn server_and_data_changes_get_the_security_review_and_product_changes_get_qa() {
        let both = |_: Reviewer| true;
        assert_eq!(needed("engineer", TicketKind::Code, &files(&["src/api/meetings.ts", "src/App.tsx"]), both), [Reviewer::Security, Reviewer::Qa]);
        assert_eq!(needed("frontend", TicketKind::Code, &files(&["src/pages/Home.tsx"]), both), [Reviewer::Qa], "a frontend is the engineer");
        assert_eq!(needed("backend", TicketKind::Code, &files(&["migrations/0003_add_slots.sql"]), both), [Reviewer::Security, Reviewer::Qa]);
        assert_eq!(needed("engineer", TicketKind::Code, &files(&["worker.js"]), both), [Reviewer::Security, Reviewer::Qa]);
        assert!(needed("engineer", TicketKind::Code, &files(&["README.md", "docs/plan.md"]), both).is_empty(), "notes need no review");
        assert!(needed("designer", TicketKind::Code, &files(&["src/App.tsx"]), both).is_empty(), "the designer's own work has design QA");
        assert!(needed("engineer", TicketKind::Task, &files(&["src/App.tsx"]), both).is_empty());
        assert_eq!(needed("engineer", TicketKind::Code, &files(&["src/api/x.ts"]), |r| r == Reviewer::Qa), [Reviewer::Qa], "no security reviewer on the squad");
    }

    #[test]
    fn server_files_are_told_from_screens() {
        for f in ["app/api/book/route.ts", "src/routes/api/+server.ts", "functions/send.js", "supabase/migrations/1.sql", "prisma/schema.prisma", "wrangler.toml", "src/lib/server/db.ts", "pages/api/x.js"] {
            assert!(server_or_data_file(f), "{f}");
        }
        for f in ["src/components/Button.tsx", "src/styles.css", "index.html", "src/pages/Apiary.tsx"] {
            assert!(!server_or_data_file(f), "{f}");
        }
    }

    #[test]
    fn a_qa_check_answer_is_read_as_its_failures() {
        let ok = r#"{"ok":true,"url":"https://x","checks":[{"key":"past_dates","ok":false,"review":true,"detail":"may accept past"}]}"#;
        assert_eq!(read_qa_check(true, ok), QaSeen::Passed, "a review finding never fails it");
        let bad = r#"{"ok":false,"url":"https://x","checks":[{"key":"legal_pages","ok":false,"detail":"no /privacy"},{"key":"rename","ok":false,"detail":"\"Settle\" still shows 14 time(s)"},{"key":"copy_numbers","ok":false,"review":true}]}"#;
        assert_eq!(read_qa_check(true, bad), QaSeen::Failed(vec!["legal_pages: no /privacy".into(), "rename: \"Settle\" still shows 14 time(s)".into()]));
        assert!(matches!(read_qa_check(false, "Error: \"app\" has no deployed copy yet"), QaSeen::Unavailable(_)));
        assert!(is_qa_check("mcp__equipe-platform__qa_check") && is_deploy("mcp__equipe-platform__deploy_site") && !is_qa_check("qa_checks_list"));
    }

    #[test]
    fn a_deployed_change_hands_in_only_after_its_checklist_passed() {
        assert!(hand_in_refused(false, None).is_none(), "nothing deployed in this work: the reviewers check it");
        assert!(hand_in_refused(true, None).unwrap().contains("Run `qa_check`"));
        assert!(hand_in_refused(true, Some(&QaSeen::Failed(vec!["rename: \"Settle\" still shows".into()]))).unwrap().contains("Settle"));
        assert!(hand_in_refused(true, Some(&QaSeen::Passed)).is_none());
        assert!(hand_in_refused(true, Some(&QaSeen::Unavailable("no deployed copy".into()))).is_none());
    }

    #[test]
    fn a_pass_without_evidence_is_refused() {
        let pass = json!({ "verdict": "pass", "findings": "All criteria hold.", "ran": ["booking flow at 390px and 1440px"], "criteria": ["past dates refused: pass"] });
        assert!(read(&json!({ "verdict": "pass", "findings": "Looks good." }), Reviewer::Security, 0, false, false, None).unwrap_err().contains("`ran`"));
        assert!(read(&json!({ "verdict": "pass", "findings": "ok", "ran": ["clicked"] }), Reviewer::Qa, 1, true, false, None).unwrap_err().contains("criterion"));
        assert!(read(&pass, Reviewer::Qa, 0, true, false, None).unwrap_err().contains("pictures"));
        assert!(read(&pass, Reviewer::Qa, 2, true, true, None).unwrap_err().contains("qa_check"));
        assert!(read(&pass, Reviewer::Qa, 2, true, true, Some(&QaSeen::Failed(vec!["legal_pages".into()]))).unwrap_err().contains("legal_pages"));
        let ok = read(&pass, Reviewer::Qa, 2, true, true, Some(&QaSeen::Passed)).unwrap();
        assert!(ok.passed && ok.findings.contains("Ran: booking flow") && ok.findings.contains("Criteria: past dates refused"));
        let changes = read(&json!({ "verdict": "changes", "findings": "POST /api/meetings takes a date in the past.", "ran": ["read the diff"] }), Reviewer::Security, 0, false, true, None).unwrap();
        assert!(!changes.passed, "a send-back needs no pictures or checklist");
    }

    #[test]
    fn the_reviewer_is_told_where_the_change_is_and_that_it_changes_nothing() {
        let change = Change { ticket: 49, title: "First version of Settle", body: "Accept: no past dates.", builder: "engineer", worktree: Some("/w/t49"), branch: Some("equipe/engineer-t49") };
        let b = brief(Reviewer::Qa, &change, 2, true);
        assert!(b.contains("git -C /w/t49 diff main...HEAD") && b.contains("round 2") && b.contains("qa_check") && b.contains("You don't change any file") && b.contains("no past dates"));
        assert!(!brief(Reviewer::Security, &Change { ticket: 1, title: "t", body: "b", builder: "e", worktree: None, branch: None }, 1, false).contains("qa_check"));
    }
}
