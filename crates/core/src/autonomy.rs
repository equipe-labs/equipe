//! The daemon's side of the squad's autonomy policy
//! ([`crate::protocol::autonomy`]): what kind a request or a question is,
//! whether a standing allowance covers a request, and the squad's store of
//! policy, allowances, uses and automatic picks (kept in its journal).
//!
//! Plain functions, tested without an engine. The runtime asks them; nothing
//! here grants anything on its own — an allowance exists only because the
//! person tapped "Allow for this kind" on a card ([`crate::protocol::Command::AllowKind`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions;
use crate::protocol::autonomy::{AutoPick, Allowance, Ask, DAY, Kind, Level, Op, PICK_KEPT, Plain, Policy, Scope, State, USES_KEPT, Use};
use crate::protocol::decide::{Question, QuestionSet};

/// A request for permission as the policy reads it.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub kind: Kind,
    pub member: &'a str,
    /// The engine's tool, or the member's own request's (`Confirm`).
    pub tool: &'a str,
    /// Everything it says it does: the command, or the member's words.
    pub action: &'a str,
    pub system: Option<&'a str>,
    pub op: Option<Op>,
}

/// What an engine's permission prompt is, as far as its tool and command
/// say: reaching outside (publishing, pushing, deploying), credentials,
/// deleting — the floor — or anything else.
pub fn engine_kind(tool: &str, action: &str) -> Kind {
    engine_kind_in(tool, action, &[])
}

/// [`engine_kind`], the squad's own `scratch` areas
/// ([`permissions::scratch_areas`]) not counted as credentials.
pub fn engine_kind_in(tool: &str, action: &str, scratch: &[String]) -> Kind {
    if permissions::outward(tool, action) {
        return Kind::Outside;
    }
    if permissions::touches_secret(action, scratch) {
        return Kind::SignIn;
    }
    if tool == "Bash" && deletes(action) {
        return Kind::Deletion;
    }
    Kind::Other
}

/// Whether a shell command deletes: `rm`, `rmdir`, `unlink`, `git clean`,
/// or a database's `DROP`/`DELETE`/`TRUNCATE`.
fn deletes(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '`' | '"' | '\'')).filter(|w| !w.is_empty()).collect();
    words.iter().any(|w| matches!(*w, "rm" | "rmdir" | "unlink" | "shred" | "truncate" | "drop" | "delete"))
        || words.windows(2).any(|w| w == ["git", "clean"])
}

/// Whether a squad on full autonomy ([`crate::protocol::autonomy::Autonomy::Full`])
/// lets an engine's permission prompt through without the person: its work
/// inside the workspace — file edits, builds, tests, dev servers, project
/// dependencies, screenshots, git without a remote — and nothing that reaches
/// outward or can't be undone. Outward commands, credentials, the floor's
/// other kinds, anything in the system, the browser's code tool and tools of
/// servers equipe doesn't know all still ask. `roots`: the folders the member
/// works in (its worktree, the squad's projects and desk); `scratch`: the
/// squad's own working areas ([`permissions::scratch_areas`]).
pub fn hands_off_allows(tool: &str, action: &str, input: &Value, kind: Kind, roots: &[String], scratch: &[String]) -> bool {
    if tool == permissions::BROWSER_CODE || permissions::outward(tool, action) || permissions::outward_more(tool, action) || permissions::touches_secret(&input.to_string(), scratch) {
        return false;
    }
    match kind {
        Kind::Other => {}
        Kind::Deletion => return tool == "Bash" && deletes_only_inside(action),
        _ => return false,
    }
    match tool {
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" | "TodoWrite" | "WebSearch" | "WebFetch" | "Task" | "Agent" => true,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => within(action, roots, scratch),
        "Bash" => !permissions::touches_system(action),
        _ => false,
    }
}

/// The environment variable a hosted team's machine sets to say the whole
/// machine is the team's sandbox (`EQUIPE_MACHINE=sandbox`).
pub const MACHINE_ENV: &str = "EQUIPE_MACHINE";

/// Whether this whole machine is the squad's sandbox: a hosted team's own
/// isolated machine, which holds nothing of the person's but the team's work.
/// Said outright with `EQUIPE_MACHINE=sandbox`; a Linux daemon on equipe
/// credits is one too (that is what a hosted team is).
pub fn sandbox_machine() -> bool {
    machine_is_sandbox(std::env::var(MACHINE_ENV).ok().as_deref(), crate::cost_levers::credits_profile(), cfg!(target_os = "linux"))
}

fn machine_is_sandbox(said: Option<&str>, credits: bool, linux: bool) -> bool {
    match said.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sandbox" | "yes" | "1" | "true") => true,
        Some("no" | "0" | "false" | "own") => false,
        _ => credits && linux,
    }
}

/// equipe's platform tools (`mcp__equipe-platform__*`) that publish, send,
/// spend or connect the person's accounts: on a sandbox machine these, and
/// only these of its tools, still ask.
pub const PLATFORM_OUTWARD: [&str; 14] = [
    "deploy_site", "promote_version", "rollback_site", "configure_email", "send_email", "buy_domain", "connect_domain",
    "enable_payments", "set_auth_client", "delete_secret", "request_machine_upgrade", "set_sprint_budget", "connect_posthog", "record_ad_spend",
];

/// The MCP servers equipe runs itself for a member, whose tools only touch
/// the machine (the browser in any profile, the board, design, a phone
/// simulator) or are its platform's ([`PLATFORM_OUTWARD`] aside).
fn equipes_own_server(server: &str) -> bool {
    matches!(server, "browser" | "equipe" | "design" | "desktop" | "phone" | "equipe-platform") || server.starts_with("browser_")
}

/// Whether, on a machine that is the squad's sandbox ([`sandbox_machine`]),
/// an engine's permission prompt goes ahead without the person. The default
/// is yes — browser code, shell steps, files anywhere on the machine,
/// packages, dev servers, screenshots, any new tool — and only an explicit
/// list of outward or irreversible things still asks: the floor's kinds
/// (credentials, money, deleting, sending or publishing, security, legal),
/// outward commands ([`permissions::outward`], [`permissions::outward_more`]),
/// anything naming a credential, the platform's outward tools
/// ([`PLATFORM_OUTWARD`]) and tools of MCP servers equipe doesn't run (the
/// person's own accounts).
pub fn sandbox_machine_allows(tool: &str, action: &str, input: &Value, kind: Kind, scratch: &[String]) -> bool {
    if kind.is_floor() || permissions::outward(tool, action) || permissions::outward_more(tool, action) || permissions::touches_secret(&input.to_string(), scratch) {
        return false;
    }
    if let Some((server, name)) = tool.strip_prefix("mcp__").and_then(|rest| rest.split_once("__")) {
        return equipes_own_server(server) && !(server == "equipe-platform" && PLATFORM_OUTWARD.contains(&name));
    }
    // The person's word (`Confirm`) or a send in their name is never the machine's to give.
    !matches!(tool, "Confirm" | "Send" | "Sign in")
}

/// Whether `path` is somewhere a member works: relative (its worktree), under
/// one of `roots` or of the squad's `scratch` areas, climbing out of none.
fn within(path: &str, roots: &[String], scratch: &[String]) -> bool {
    if path.is_empty() || path.contains("..") || path.contains(['$', '`']) {
        return false;
    }
    if !path.starts_with(['/', '~']) {
        return true;
    }
    roots.iter().any(|r| path.strip_prefix(r.trim_end_matches('/')).is_some_and(|rest| rest.starts_with('/'))) || scratch.iter().any(|d| path.starts_with(d.as_str()))
}

/// Whether every deleting command of a chain is a plain `rm`, `rmdir` or
/// `unlink` of named things in the workspace: relative paths, climbing out of
/// nowhere, not the folder itself, a bare glob, `.git` or a variable. A
/// deletion of anything else — a database, `git clean`, a path elsewhere — is
/// the person's to allow.
fn deletes_only_inside(command: &str) -> bool {
    permissions::commands(command).iter().filter(|c| deletes(c)).all(|c| {
        let words: Vec<&str> = c.split_whitespace().collect();
        let targets: Vec<&str> = words.iter().skip(1).filter(|w| !w.starts_with('-')).map(|w| w.trim_matches(['"', '\''])).collect();
        matches!(words.first(), Some(&("rm" | "rmdir" | "unlink")))
            && !targets.is_empty()
            && targets.iter().all(|t| {
                !t.starts_with(['/', '~']) && !t.contains(['$', '`', '*', '?', '[']) && !t.contains("..") && !matches!(*t, "." | "./" | "") && !t.trim_start_matches("./").starts_with(".git")
            })
    })
}

/// What a member's own request (`ask_person`) is: a floor kind it names in
/// its words, else what it says it is (production data or other). A request
/// that offers to delete what it made within days (`own_deletes`, its
/// standing scope's) may speak of deleting without being a deletion.
pub fn person_kind(declared: Option<Kind>, words: &str, own_deletes: bool) -> Kind {
    if let Some(floor) = declared.filter(|k| k.is_floor()) {
        return floor;
    }
    let except = own_deletes.then_some(Kind::Deletion);
    if let Some(floor) = Kind::floor_named(words, except) {
        return floor;
    }
    match declared {
        Some(Kind::ProductionData) => Kind::ProductionData,
        _ => Kind::Other,
    }
}

/// The standing allowance that covers `req`, if any: live, of its kind, and
/// in its scope — an engine rule that matches, or a labeled write on its
/// system, carrying its marker, doing one of its ops, naming nothing on the
/// floor; a delete only of what the same member made under it within its days.
pub fn covering<'a>(allowances: &'a [Allowance], uses: &[Use], req: &Request, now: u64) -> Option<&'a Allowance> {
    if req.kind.is_floor() {
        return None;
    }
    allowances.iter().filter(|a| a.live(now) && a.kind == req.kind).find(|a| covers(a, uses, req, now))
}

fn covers(a: &Allowance, uses: &[Use], req: &Request, now: u64) -> bool {
    if let Some(rule) = a.scope.rule.as_deref() {
        return engine_kind(req.tool, req.action) == Kind::Other && permissions::matches(rule, req.tool, req.action);
    }
    let (Some(system), Some(marker), Some(op)) = (a.scope.system.as_deref(), a.scope.marker.as_deref(), req.op) else { return false };
    let lower = req.action.to_lowercase();
    let on_system = req.system.is_some_and(|s| s.trim().eq_ignore_ascii_case(system.trim())) || lower.contains(&system.trim().to_lowercase());
    let marked = lower.contains(&marker.trim().to_lowercase());
    let own_deletes = a.scope.ops.contains(&Op::Delete) && a.scope.undo_days > 0;
    if !on_system || !marked || !a.scope.ops.contains(&op) || Kind::floor_named(req.action, own_deletes.then_some(Kind::Deletion)).is_some() {
        return false;
    }
    if op == Op::Delete {
        let since = now.saturating_sub(u64::from(a.scope.undo_days) * DAY);
        return own_deletes && uses.iter().any(|u| u.allowance == a.id && u.member == req.member && u.op == Some(Op::Create) && u.at >= since);
    }
    true
}

/// What a rule allows, in short: `npm install`, `docs.rs`, a folder.
pub fn rule_label(rule: &str) -> String {
    let inner = rule.split_once('(').and_then(|(_, r)| r.strip_suffix(')')).unwrap_or(rule);
    let short = inner.trim_end_matches(":*").trim_start_matches("domain:");
    match short.strip_prefix('/').and_then(|p| p.strip_suffix("/**")) {
        Some(dir) => format!("/{dir}/"),
        None => short.to_owned(),
    }
}

/// The standing scope a member's request offers, read from `ask_person`'s
/// `standing` and `system`: sound, or none.
pub fn offered_scope(args: &Value, system: Option<&str>) -> Option<Scope> {
    let standing = args.get("standing")?;
    let text = |key: &str| standing.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    let ops: Vec<Op> = standing.get("ops").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(Op::parse).fold(Vec::new(), |mut ops, op| {
        if !ops.contains(&op) {
            ops.push(op);
        }
        ops
    });
    let undo_days = standing.get("undo_days").and_then(Value::as_u64).map_or(0, |d| d.min(u64::from(crate::protocol::autonomy::MOST_DAYS)) as u32);
    let scope = Scope {
        words: text("words").unwrap_or_default(),
        rule: None,
        system: system.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned),
        marker: text("marker"),
        ops,
        undo_days: if undo_days > 0 { undo_days } else { 0 },
    };
    scope.is_sound().then_some(scope)
}

/// The plain words a member's request leads with (`summary`), if it gave them.
pub fn plain_of(args: &Value) -> Option<Plain> {
    let summary = args.get("summary")?;
    let text = |key: &str| summary.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let plain = Plain { changes: text("changes"), who_sees: text("who_sees"), undo: text("undo") };
    (!plain.is_empty()).then_some(plain)
}

/// What a question set is, for the policy: a floor kind its words name, the
/// squad's own process when the member said so of every question, a design
/// choice when it is one (its options pictured or tried, or a designer's) or
/// a product choice with the member's recommendation (decided and told the
/// same way), or anything else — asked.
pub fn set_kind(set: &QuestionSet, role: &str, declared: &[Option<Kind>]) -> Kind {
    let words: Vec<String> = set.questions.iter().flat_map(|q| std::iter::once(q.question.clone()).chain(q.options.iter().map(|c| format!("{} {}", c.label, c.description)))).collect();
    if let Some(floor) = Kind::floor_named(&words.join("\n"), None) {
        return floor;
    }
    let said = |kind: Kind| !declared.is_empty() && set.questions.iter().enumerate().all(|(n, _)| declared.get(n).copied().flatten() == Some(kind));
    if said(Kind::Internal) {
        return Kind::Internal;
    }
    let designed = |q: &Question| q.is_visual() || q.tryable() || declared.contains(&Some(Kind::Design));
    if said(Kind::Design) || role == "designer" || set.questions.iter().all(designed) {
        return Kind::Design;
    }
    // A product choice the member can recommend an answer to — nothing on the
    // floor in it, nothing it says touches live data — is reversible: decided
    // and told like a design choice, the person free to change it after.
    let recommends = |q: &Question| q.options.len() >= 2 && !q.board && q.options.iter().any(|c| c.shown().1);
    if !declared.contains(&Some(Kind::ProductionData)) && !set.questions.is_empty() && set.questions.iter().all(recommends) {
        return Kind::Design;
    }
    Kind::Other
}

/// A choice waiting to be taken on its member's recommendation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Due {
    pub set: u64,
    pub kind: Kind,
    pub at: u64,
}

/// A squad's autonomy as its journal keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub policy: Policy,
    /// How the squad is let to work ([`crate::protocol::autonomy::Autonomy`]).
    #[serde(default, skip_serializing_if = "crate::protocol::autonomy::Autonomy::is_default")]
    pub mode: crate::protocol::autonomy::Autonomy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowances: Vec<Allowance>,
    /// Oldest first, the latest [`USES_KEPT`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Use>,
    /// Oldest first, for [`PICK_KEPT`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<AutoPick>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub due: Vec<Due>,
    #[serde(default)]
    pub next_id: u64,
}

impl Store {
    /// Grant a standing allowance for `days` (within 1 and the most), kept
    /// with what it was granted on. Its id.
    pub fn grant(&mut self, kind: Kind, scope: Scope, from: &str, days: u32, now: u64) -> Option<u64> {
        if !kind.allowable() || !scope.is_sound() {
            return None;
        }
        self.next_id = self.next_id.max(self.allowances.iter().map(|a| a.id).max().unwrap_or(0)) + 1;
        let days = days.clamp(1, crate::protocol::autonomy::MOST_DAYS);
        let id = self.next_id;
        self.allowances.push(Allowance { id, kind, scope, from: from.to_owned(), granted_at: now, expires_at: now + u64::from(days) * DAY, revoked_at: None, uses: 0 });
        Some(id)
    }

    /// Take allowance `id` back. Whether it was live.
    pub fn revoke(&mut self, id: u64, now: u64) -> bool {
        match self.allowances.iter_mut().find(|a| a.id == id && a.revoked_at.is_none()) {
            Some(a) => {
                a.revoked_at = Some(now);
                true
            }
            None => false,
        }
    }

    /// Log a use of allowance `use_.allowance`.
    pub fn record(&mut self, use_: Use) {
        if let Some(a) = self.allowances.iter_mut().find(|a| a.id == use_.allowance) {
            a.uses += 1;
        }
        self.uses.push(use_);
        let over = self.uses.len().saturating_sub(USES_KEPT);
        self.uses.drain(..over);
    }

    /// Let go of what has run its course: picks past their week, allowances
    /// a month past their end, uses of allowances let go.
    pub fn tidy(&mut self, now: u64) {
        self.picks.retain(|p| now.saturating_sub(p.at) < PICK_KEPT);
        self.allowances.retain(|a| now.saturating_sub(a.revoked_at.unwrap_or(a.expires_at)) < 30 * DAY);
        let kept: Vec<u64> = self.allowances.iter().map(|a| a.id).collect();
        self.uses.retain(|u| kept.contains(&u.allowance));
    }

    /// What screens show, the merge level as the squad has it (`pm_accepts`).
    pub fn state(&self, pm_accepts: bool) -> State {
        let mut policy = self.policy.clone();
        policy.merge = match (pm_accepts, policy.merge) {
            (false, _) => Level::Ask,
            (true, Level::Ask) => Level::JustDo,
            (true, level) => level,
        };
        State {
            policy,
            mode: self.mode,
            allowances: self.allowances.iter().rev().cloned().collect(),
            uses: self.uses.iter().rev().take(50).cloned().collect(),
            picks: self.picks.iter().rev().map(AutoPick::view).collect(),
            due: self.due.iter().map(|d| crate::protocol::autonomy::DueView { set: d.set, at: d.at }).collect(),
        }
    }
}

/// Whether `ask` lets the request be allowed without the person on the
/// policy alone (no allowance): production data set to decide-and-tell or
/// just-do. Never the floor, never "other".
pub fn policy_allows(policy: &Policy, ask: &Ask) -> bool {
    ask.kind == Kind::ProductionData && policy.level(Kind::ProductionData) != Level::Ask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::autonomy::Scope;
    use crate::protocol::decide::{Choice, Question};
    use serde_json::json;

    const NOW: u64 = 1_000_000;

    fn erp() -> Scope {
        Scope { words: "Test records in ERP".into(), system: Some("ERP".into()), marker: Some("[test]".into()), ops: vec![Op::Create, Op::Delete], undo_days: 7, ..Scope::default() }
    }

    fn req<'a>(member: &'a str, action: &'a str, op: Op) -> Request<'a> {
        Request { kind: Kind::ProductionData, member, tool: "Confirm", action, system: Some("ERP"), op: Some(op) }
    }

    #[test]
    fn the_squads_scratch_is_not_credentials_for_lying_under_dot_equipe() {
        let scratch = permissions::scratch_areas(std::path::Path::new("/home/tenki/.equipe"), "team");
        let edit = "/home/tenki/.equipe/tmp/team/t1/shoot.mjs";
        assert_eq!(engine_kind("Edit", edit), Kind::SignIn, "unnarrowed it is asked as a key");
        assert_eq!(engine_kind_in("Edit", edit, &scratch), Kind::Other);
        assert_eq!(engine_kind_in("Edit", "/home/tenki/.equipe/config.json", &scratch), Kind::SignIn);
        assert_eq!(engine_kind_in("Bash", "cat /home/tenki/.equipe/tmp/other/t1/a", &scratch), Kind::SignIn);
    }

    #[test]
    fn full_autonomy_lets_internal_work_through_and_keeps_the_rest_for_the_person() {
        let scratch = permissions::scratch_areas(std::path::Path::new("/home/tenki/.equipe"), "team");
        let roots = ["/home/tenki/work/site".to_owned(), "/home/tenki/.equipe/squads/team/home".to_owned()];
        let allows = |tool: &str, action: &str| {
            let input = if tool == "Bash" { json!({ "command": action }) } else { json!({ "file_path": action }) };
            hands_off_allows(tool, action, &input, engine_kind_in(tool, action, &scratch), &roots, &scratch)
        };
        for (tool, action) in [
            ("Bash", "npm install"), ("Bash", "npm run build && npm test"), ("Bash", "pnpm dev"), ("Bash", "git add -A && git commit -m x"), ("Bash", "git checkout -b fix"),
            ("Bash", "node /home/tenki/.equipe/tmp/team/t1/shoot.mjs"), ("Bash", "pip install -r requirements.txt"), ("Bash", "rm -rf node_modules dist"),
            ("Bash", "curl https://example.com/a.json -o a.json"), ("Write", "/home/tenki/work/site/index.html"), ("Edit", "src/App.tsx"),
            ("Write", "/home/tenki/.equipe/tmp/team/t1/shoot.mjs"), ("Edit", "/home/tenki/.equipe/squads/team/home/notes/plan.md"), ("Read", "/anywhere/readme.md"),
            ("WebFetch", "https://example.com"),
        ] {
            assert!(allows(tool, action), "{tool} {action}");
        }
        for (tool, action) in [
            ("Bash", "git push origin main"), ("Bash", "gh pr create"), ("Bash", "vercel deploy"), ("Bash", "npx wrangler deploy"), ("Bash", "curl -X POST https://x.test -d a=b"),
            ("Bash", "rm -rf /home/tenki/Documents"), ("Bash", "rm -rf ../photos"), ("Bash", "rm -rf ."), ("Bash", "rm -rf *"), ("Bash", "rm -rf .git"), ("Bash", "git clean -fdx"),
            ("Bash", "psql -c 'DROP TABLE users'"), ("Bash", "cat ~/.ssh/id_rsa"), ("Bash", "cat /home/tenki/.equipe/config.json"), ("Bash", "cp a /etc/hosts"),
            ("Bash", "npm install && npm publish"), ("Write", "/etc/hosts"), ("Write", "/home/tenki/other-project/a.txt"), ("Write", "/home/tenki/work/site/../../.bashrc"),
            ("Write", "/home/tenki/.equipe/config.json"), ("Write", "/home/tenki/.equipe/tmp/other/t1/a"), ("Edit", "/home/tenki/work/site/.env"),
            (permissions::BROWSER_CODE, "async (page) => page.click('#buy')"), ("mcp__gmail__send_message", "{}"),
        ] {
            assert!(!allows(tool, action), "{tool} {action}");
        }
    }

    #[test]
    fn on_a_sandbox_machine_only_the_outward_list_asks_and_anything_new_goes_ahead() {
        let scratch = permissions::scratch_areas(std::path::Path::new("/home/tenki/.equipe"), "team");
        let allows = |tool: &str, action: &str| {
            let input = match tool {
                "Bash" => json!({ "command": action }),
                t if t.contains("run_code") => json!({ "code": action }),
                t if t.starts_with("mcp__") => serde_json::from_str(action).unwrap_or(json!({})),
                _ => json!({ "file_path": action }),
            };
            sandbox_machine_allows(tool, action, &input, engine_kind_in(tool, action, &scratch), &scratch)
        };
        // The team's own machine: browser code in any profile, system installs, files anywhere, and any tool
        // equipe hasn't heard of yet (a new built-in, a new tool of its own servers) — never a card.
        for (tool, action) in [
            (permissions::BROWSER_CODE, "async (page) => { await page.goto('http://127.0.0.1:4318/'); return page.title(); }"),
            ("mcp__browser_jhjung__browser_run_code_unsafe", "async (page) => page.screenshot()"),
            ("mcp__browser__browser_take_screenshot", "{}"), ("Bash", "sudo apt-get install -y ffmpeg"), ("Bash", "npm run dev &"),
            ("Bash", "ls /var/log && cat /etc/os-release"), ("Write", "/etc/hosts"), ("Edit", "/home/tenki/other/a.txt"), ("Read", "/anywhere"),
            ("mcp__equipe-platform__generate_image", "{}"), ("mcp__equipe-platform__a_tool_added_tomorrow", "{}"), ("mcp__design__palette", "{}"),
            ("SomeNewBuiltinTool", "{}"), ("WebFetch", "https://example.com"),
        ] {
            assert!(allows(tool, action), "{tool} {action}");
        }
        // The explicit outward list: still the person's.
        for (tool, action) in [
            ("Bash", "git push origin main"), ("Bash", "vercel deploy --prod"), ("Bash", "npx wrangler deploy"), ("Bash", "curl -X POST https://x.test -d a=b"),
            ("Bash", "rm -rf /home/tenki/Documents"), ("Bash", "cat ~/.aws/credentials"), ("Read", "/home/tenki/.equipe/config.json"), ("Write", "/home/tenki/.claude/settings.json"),
            ("Edit", "/home/tenki/work/site/.env"), ("mcp__equipe-platform__send_email", "{}"), ("mcp__equipe-platform__deploy_site", "{}"),
            ("mcp__equipe-platform__buy_domain", "{}"), ("mcp__gmail__send_message", "{}"), ("Confirm", "Email the waitlist"), ("Send", "hi"),
        ] {
            assert!(!allows(tool, action), "{tool} {action}");
        }
        // Every floor kind asks, whatever the tool.
        for kind in Kind::FLOOR {
            assert!(!sandbox_machine_allows("SomeNewBuiltinTool", "x", &json!({}), kind, &scratch), "{kind:?}");
        }
    }

    #[test]
    fn a_hosted_linux_team_on_credits_is_a_sandbox_machine_and_a_mac_is_not() {
        assert!(machine_is_sandbox(None, true, true));
        assert!(!machine_is_sandbox(None, true, false), "a Mac on credits is the person's own computer");
        assert!(!machine_is_sandbox(None, false, true));
        assert!(machine_is_sandbox(Some("sandbox"), false, false));
        assert!(!machine_is_sandbox(Some("own"), true, true));
    }

    #[test]
    fn a_labeled_allowance_covers_marked_writes_on_its_system_and_nothing_else() {
        let mut store = Store::default();
        let id = store.grant(Kind::ProductionData, erp(), "qa-2", 7, NOW).unwrap();
        let a = &store.allowances;
        let create = "Create one unsubmitted seal-register record titled '[test] link check' in the ERP";
        assert_eq!(covering(a, &[], &req("qa-2", create, Op::Create), NOW).map(|a| a.id), Some(id));
        assert_eq!(covering(a, &[], &req("backend", create, Op::Create), NOW).map(|a| a.id), Some(id), "any member may use it");
        // Unmarked, another op, another system, or the floor named: asked.
        assert!(covering(a, &[], &req("qa-2", "Create a seal-register record in the ERP", Op::Create), NOW).is_none(), "no marker");
        assert!(covering(a, &[], &req("qa-2", create, Op::Update), NOW).is_none(), "not one of its ops");
        let crm = Request { system: Some("CRM"), ..req("qa-2", "Create a [test] record in the CRM", Op::Create) };
        assert!(covering(a, &[], &crm, NOW).is_none(), "another system");
        assert!(covering(a, &[], &req("qa-2", "Create a [test] record in the ERP and email the approvers", Op::Create), NOW).is_none(), "sends outside");
        let money = Request { kind: Kind::Money, ..req("qa-2", create, Op::Create) };
        assert!(covering(a, &[], &money, NOW).is_none(), "the floor never matches");
        // It speaks of deleting later: its own deletes are in scope, so that doesn't stop it.
        assert!(covering(a, &[], &req("qa-2", "Create a [test] record in the ERP — to be deleted after the check", Op::Create), NOW).is_some());
    }

    #[test]
    fn a_delete_is_covered_only_for_what_the_same_member_made_within_the_days() {
        let mut store = Store::default();
        let id = store.grant(Kind::ProductionData, erp(), "qa-2", 30, NOW).unwrap();
        let made = Use { allowance: id, member: "qa-2".into(), ticket: Some(5), op: Some(Op::Create), what: "[test] record".into(), at: NOW };
        store.record(made.clone());
        let delete = "Delete the [test] link check record from the ERP";
        let (a, uses) = (&store.allowances, &store.uses);
        assert!(covering(a, uses, &req("qa-2", delete, Op::Delete), NOW + DAY).is_some(), "its own, within the week");
        assert!(covering(a, uses, &req("backend", delete, Op::Delete), NOW + DAY).is_none(), "another member's");
        assert!(covering(a, uses, &req("qa-2", delete, Op::Delete), NOW + 8 * DAY).is_none(), "past the days");
        assert!(covering(a, &[], &req("qa-2", delete, Op::Delete), NOW).is_none(), "nothing made");
        // Without delete in its ops, a delete names the floor.
        let mut create_only = Store::default();
        let only = create_only.grant(Kind::ProductionData, Scope { ops: vec![Op::Create], undo_days: 0, ..erp() }, "qa-2", 7, NOW).unwrap();
        create_only.record(Use { allowance: only, ..made });
        assert!(covering(&create_only.allowances, &create_only.uses, &req("qa-2", delete, Op::Delete), NOW).is_none());
    }

    #[test]
    fn an_allowance_ends_when_it_expires_or_is_revoked() {
        let mut store = Store::default();
        let id = store.grant(Kind::ProductionData, erp(), "qa-2", 7, NOW).unwrap();
        let create = "Create a [test] record in the ERP";
        assert!(covering(&store.allowances, &[], &req("qa-2", create, Op::Create), NOW + 7 * DAY - 1).is_some());
        assert!(covering(&store.allowances, &[], &req("qa-2", create, Op::Create), NOW + 7 * DAY).is_none(), "expired");
        assert!(store.revoke(id, NOW + 1));
        assert!(!store.revoke(id, NOW + 2), "once");
        assert!(covering(&store.allowances, &[], &req("qa-2", create, Op::Create), NOW + 2).is_none(), "revoked");
        // Days are kept within bounds; the floor and unsound scopes are never granted.
        let long = store.grant(Kind::ProductionData, erp(), "qa-2", 10_000, NOW).unwrap();
        assert_eq!(store.allowances.iter().find(|a| a.id == long).unwrap().expires_at, NOW + 90 * DAY);
        assert_eq!(store.grant(Kind::Outside, erp(), "qa-2", 7, NOW), None);
        assert_eq!(store.grant(Kind::ProductionData, Scope { marker: None, ..erp() }, "qa-2", 7, NOW), None);
    }

    #[test]
    fn a_rule_allowance_covers_its_commands_but_never_the_floor() {
        let mut store = Store::default();
        let scope = Scope { words: "npm test".into(), rule: Some("Bash(npm test:*)".into()), ..Scope::default() };
        store.grant(Kind::Other, scope, "frontend", 7, NOW).unwrap();
        let bash = |action| Request { kind: Kind::Other, member: "frontend", tool: "Bash", action, system: None, op: None };
        assert!(covering(&store.allowances, &[], &bash("npm test -- --watch=false"), NOW).is_some());
        assert!(covering(&store.allowances, &[], &bash("npm publish"), NOW).is_none());
        let broad = Scope { words: "gh".into(), rule: Some("Bash(gh:*)".into()), ..Scope::default() };
        store.grant(Kind::Other, broad, "pm", 7, NOW).unwrap();
        assert!(covering(&store.allowances, &[], &bash("gh issue create --title x"), NOW).is_none(), "outward stays asked");
    }

    #[test]
    fn engine_prompts_and_member_requests_are_read_for_the_floor() {
        assert_eq!(engine_kind("Bash", "git push origin main"), Kind::Outside);
        assert_eq!(engine_kind("Bash", "cat ~/.aws/credentials"), Kind::SignIn);
        assert_eq!(engine_kind("Bash", "rm -r build/cache"), Kind::Deletion);
        assert_eq!(engine_kind("Bash", "psql -c 'DROP TABLE users'"), Kind::Deletion);
        assert_eq!(engine_kind("Bash", "npm install"), Kind::Other);
        assert_eq!(person_kind(Some(Kind::ProductionData), "Create a [test] record in the ERP", false), Kind::ProductionData);
        assert_eq!(person_kind(Some(Kind::ProductionData), "Sign up for PostHog and accept its terms", false), Kind::SignIn, "its words outrank its label");
        assert_eq!(person_kind(Some(Kind::Money), "anything", false), Kind::Money);
        assert_eq!(person_kind(None, "Restart the daemon on this Mac", false), Kind::Other);
        assert_eq!(person_kind(Some(Kind::Design), "Restart the daemon", false), Kind::Other, "not a request's kind");
        let later = "Create a [test] record in the ERP, to be deleted after the check";
        assert_eq!(person_kind(Some(Kind::ProductionData), later, false), Kind::Deletion);
        assert_eq!(person_kind(Some(Kind::ProductionData), later, true), Kind::ProductionData, "its own deletes offered");
    }

    #[test]
    fn a_member_offers_a_scope_only_when_it_is_sound_and_its_plain_words_when_given() {
        let args = json!({ "standing": { "words": "Test records marked [test] in the ERP", "marker": "[test]", "ops": ["create", "delete", "create"], "undo_days": 7 } });
        let scope = offered_scope(&args, Some("ERP")).unwrap();
        assert_eq!((scope.ops.clone(), scope.undo_days, scope.marker.as_deref()), (vec![Op::Create, Op::Delete], 7, Some("[test]")));
        assert_eq!(offered_scope(&args, None), None, "no system");
        assert_eq!(offered_scope(&json!({ "standing": { "marker": "x", "ops": ["create"] } }), Some("ERP")), None, "a marker too short");
        assert_eq!(offered_scope(&json!({}), Some("ERP")), None);
        assert_eq!(plain_of(&json!({ "summary": { "changes": "One test record", "who_sees": "Only admins", "undo": "Deleted within 7 days" } })).map(|p| p.who_sees), Some("Only admins".into()));
        assert_eq!(plain_of(&json!({ "summary": { "changes": " " } })), None);
    }

    fn q(options: Vec<Choice>) -> Question {
        Question { question: "Which header layout?".into(), options, ..Question::default() }
    }

    fn set(questions: Vec<Question>) -> QuestionSet {
        QuestionSet { id: 1, session: 2, member: "frontend".into(), ticket: None, questions, asked_at: 0, review: false, retro: None }
    }

    #[test]
    fn a_question_set_is_a_design_choice_internal_or_asked() {
        let plain = vec![Choice { label: "A".into(), ..Choice::default() }, Choice { label: "B".into(), recommended: true, ..Choice::default() }];
        let pictured: Vec<Choice> = plain.iter().map(|c| Choice { images: vec!["/a.png".into()], ..c.clone() }).collect();
        assert_eq!(set_kind(&set(vec![q(pictured)]), "frontend", &[]), Kind::Design, "pictured options");
        assert_eq!(set_kind(&set(vec![q(plain.clone())]), "designer", &[]), Kind::Design, "a designer's");
        assert_eq!(set_kind(&set(vec![q(plain.clone())]), "frontend", &[]), Kind::Design, "a product choice with a recommendation: decide-and-tell");
        let unrecommended: Vec<Choice> = plain.iter().map(|c| Choice { recommended: false, ..c.clone() }).collect();
        assert_eq!(set_kind(&set(vec![q(unrecommended)]), "frontend", &[]), Kind::Other, "a product question without one: asked");
        assert_eq!(set_kind(&set(vec![q(plain.clone())]), "backend", &[Some(Kind::ProductionData)]), Kind::Other, "live data: asked");
        assert_eq!(set_kind(&set(vec![q(plain.clone())]), "frontend", &[Some(Kind::Internal)]), Kind::Internal);
        assert_eq!(set_kind(&set(vec![q(plain.clone()), q(plain.clone())]), "frontend", &[Some(Kind::Internal)]), Kind::Design, "not every question is internal: recommended choices, decided and told");
        let legal = Question { question: "Is the licence of this icon set OK for us?".into(), ..q(plain) };
        assert_eq!(set_kind(&set(vec![legal]), "designer", &[]), Kind::Legal, "the floor wins");
    }

    #[test]
    fn the_store_keeps_its_logs_short_and_shows_the_merge_level_the_squad_has() {
        let mut store = Store::default();
        let id = store.grant(Kind::ProductionData, erp(), "qa-2", 1, NOW).unwrap();
        for n in 0..(USES_KEPT + 5) {
            store.record(Use { allowance: id, member: "qa-2".into(), ticket: None, op: Some(Op::Create), what: format!("{n}"), at: NOW });
        }
        assert_eq!(store.uses.len(), USES_KEPT);
        assert_eq!(store.allowances[0].uses as usize, USES_KEPT + 5);
        assert_eq!(store.state(true).uses.len(), 50);
        assert_eq!(store.state(true).uses[0].what, format!("{}", USES_KEPT + 4), "latest first");
        assert_eq!(store.state(false).policy.merge, Level::Ask, "the person keeps acceptances");
        assert_eq!(store.state(true).policy.merge, Level::JustDo);
        // A month after it ended, an allowance and its uses go.
        store.tidy(NOW + 31 * DAY + DAY);
        assert!(store.allowances.is_empty() && store.uses.is_empty());
        // It reads back from the journal's JSON.
        let json = serde_json::to_string(&Store::default()).unwrap();
        assert_eq!(serde_json::from_str::<Store>(&json).unwrap(), Store::default());
    }

    #[test]
    fn only_production_data_set_past_ask_is_allowed_on_the_policy_alone() {
        let mut p = Policy::default();
        let ask = |kind| Ask { kind, plain: None, standing: None, later: false, op: None };
        assert!(!policy_allows(&p, &ask(Kind::ProductionData)));
        p.set(Kind::ProductionData, Level::Tell).unwrap();
        assert!(policy_allows(&p, &ask(Kind::ProductionData)));
        assert!(!policy_allows(&p, &ask(Kind::Other)));
        assert!(!policy_allows(&p, &ask(Kind::Deletion)));
    }
}
