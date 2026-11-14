//! Keys, tokens and passwords a squad's members obtain — a PostHog project
//! key from a dashboard, a token a CLI printed, a database password — kept so
//! a later ticket can use them, instead of living only in some host's
//! environment variables.
//!
//! - The **value** lives only in the machine's secret store, in the same
//!   Keychain item as the logins: `equipe.secrets:<squad>/<name>`
//!   ([`crate::vault::Vault::set_squad_secret`]). Never in a file, an event,
//!   a prompt or a log.
//! - The **facts** about it — name, kind, service, the projects that may use
//!   it, who saved it and when, its last use — are the squad's journal's
//!   (`meta`, as JSON): [`read`], [`put`].
//! - A member **saves** one (`secret_save`), **lists** what there is
//!   (`secret_list`, facts only), and **uses** one (`secret_use`: an
//!   environment variable of its next turns; `secret_write`: a line in a
//!   git-ignored file of its worktree). Neither answers with the value.
//! - A secret is usable only by its squad's members and, on a project, only
//!   where the person allowed it: the first use on a new project is a card
//!   the person answers once (`Access::Ask`), unless they marked it for any
//!   project. Every use is recorded ([`record`]).
//! - Every stored value is **masked** wherever text leaves a member:
//!   [`mask`] cuts each one out verbatim (and its JSON-escaped form), from
//!   events, ticket text, hand-ins, memory and the daemon's log. What looks
//!   like a key but wasn't saved is spotted ([`keys_in`], [`scrub`]) so the
//!   member can be told to save it.
//!
//! Nothing here sends anything off the machine.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Value, json};

use crate::journal::Journal;
use crate::protocol::TicketId;
use crate::protocol::secrets::{NO_PROJECT, SecretKind, SecretUse, SquadSecret};

/// The service the values are kept under, in the Keychain item.
pub const SERVICE: &str = "equipe.secrets";

pub const SAVE: &str = "secret_save";
pub const LIST: &str = "secret_list";
pub const USE: &str = "secret_use";
pub const WRITE: &str = "secret_write";

/// The longest value kept: a private key fits; a file doesn't.
pub const MOST_BYTES: usize = 16 * 1024;
/// Uses kept in the journal, the latest.
const USES_KEPT: usize = 500;

/// The account a secret is kept under inside [`SERVICE`].
pub fn account(squad: &str, name: &str) -> String {
    format!("{squad}/{name}")
}

/// A secret's name: letters, digits, `.`, `_` and `-`, starting with a letter or digit.
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.starts_with(|c: char| c.is_ascii_alphanumeric()) && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

// ───────────────────────── the tools ─────────────────────────

/// The four tools, as MCP tool definitions.
pub fn tools() -> Vec<Value> {
    let kinds: Vec<&str> = SecretKind::ALL.iter().map(|k| k.code()).collect();
    vec![
        json!({
            "name": SAVE,
            "description": "Keep a key, token, password or connection string you just obtained — a project API key from a \
dashboard, a token a CLI printed, a database password — so you or a teammate can use it on a later ticket. equipe stores it in \
this machine's secret store; this tool never repeats the value, and from now on equipe masks it in transcripts, tickets, \
memory and hand-ins. Name it for what it is (`posthog-project-key`). Never paste a key into code, a ticket, a comment, memory or \
chat — save it here. Saving a name again replaces its value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "What to call it: letters, digits, . _ - (`posthog-project-key`)." },
                    "service": { "type": "string", "description": "The service or site it is for: `posthog`, `vercel`, `app.posthog.com`." },
                    "kind": { "type": "string", "enum": kinds, "description": "What it is." },
                    "value": { "type": "string", "description": "The key itself." },
                    "projects": { "type": "array", "items": { "type": "string" }, "description": "The squad's projects whose work may use it, by name. Left out: the project of your ticket. Anything else asks the person on first use." },
                    "note": { "type": "string", "description": "A line on what it is for or where it came from — never the value." },
                },
                "required": ["name", "service", "kind", "value"],
            },
        }),
        json!({
            "name": LIST,
            "description": "The keys, tokens and passwords equipe keeps for this squad: name, kind, service, the projects they \
may be used on, who saved them and when, and their last use. Never a value.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": USE,
            "description": "Make a kept key available to your commands as an environment variable: from your next turn on, \
every command you run has it (`as_env`, `$POSTHOG_KEY`), and the value is never returned as text. For a key you need in \
this turn, use `secret_write`. The squad's members only, and only on the projects the person allowed: the first use on \
a new project is put to them on a card — you are told, and ask again once they answer. Never print, echo or copy the \
value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "A kept key's name (`secret_list`)." },
                    "as_env": { "type": "string", "description": "The environment variable it is given as: uppercase letters, digits and _ (`POSTHOG_KEY`)." },
                },
                "required": ["name", "as_env"],
            },
        }),
        json!({
            "name": WRITE,
            "description": "Write a kept key into a file in your worktree, for a program that reads it from there \
(`.env.local`). The file must be git-ignored and untracked, or this refuses — add it to `.gitignore` first. The file is \
readable by the person only; the value is never returned as text. Same rule as `secret_use` for who and where.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "A kept key's name (`secret_list`)." },
                    "path": { "type": "string", "description": "The file, relative to your worktree: `.env.local`." },
                    "format": { "type": "string", "enum": ["env", "json", "raw"], "description": "`env` (default): a `KEY=\"value\"` line, other lines kept. `json`: a field of a JSON object. `raw`: the file is the value." },
                    "key": { "type": "string", "description": "The variable (env: uppercase, digits, _) or field (json) it is written as. Left out: the secret's name in capitals for env, its name for json. Not used by raw." },
                },
                "required": ["name", "path"],
            },
        }),
    ]
}

/// A tool's name, whichever way the engine spells it (`mcp__equipe__secret_save`).
pub fn is_tool(tool: &str, which: &str) -> bool {
    tool == which || tool.ends_with(&format!("__{which}"))
}

/// Whether a tool is one whose arguments hold a value.
pub fn carries_value(tool: &str) -> bool {
    is_tool(tool, SAVE)
}

// ───────────────────────── the facts ─────────────────────────

/// The squad's kept keys, by name.
pub fn read(journal: &Journal) -> Vec<SquadSecret> {
    let mut all: Vec<SquadSecret> = journal.squad_secrets().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

fn keep(journal: &Journal, all: &[SquadSecret]) -> Result<(), String> {
    journal.set_squad_secrets(&serde_json::to_string(all).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Add `secret`, or replace the one of its name.
pub fn put(journal: &Journal, secret: SquadSecret) -> Result<(), String> {
    let mut all = read(journal);
    match all.iter_mut().find(|s| s.name == secret.name) {
        Some(found) => *found = secret,
        None => all.push(secret),
    }
    keep(journal, &all)
}

/// Take `name` out; whether it was there.
pub fn remove(journal: &Journal, name: &str) -> Result<bool, String> {
    let mut all = read(journal);
    let before = all.len();
    all.retain(|s| s.name != name);
    keep(journal, &all)?;
    Ok(all.len() != before)
}

/// The person's change: exactly these projects, or any.
pub fn set_scope(journal: &Journal, name: &str, projects: &[String], any_project: bool) -> Result<(), String> {
    let mut all = read(journal);
    let found = all.iter_mut().find(|s| s.name == name).ok_or_else(|| format!("no key called {name}"))?;
    found.projects = projects.iter().map(|p| p.trim().to_owned()).fold(Vec::new(), |mut out, p| {
        if !out.contains(&p) {
            out.push(p);
        }
        out
    });
    found.any_project = any_project;
    keep(journal, &all)
}

/// Whether a secret may be used on a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// The first use here: the person is asked once.
    Ask,
}

pub fn access(secret: &SquadSecret, project: &str) -> Access {
    if secret.any_project || secret.projects.iter().any(|p| p == project) { Access::Allowed } else { Access::Ask }
}

/// The person lets `project` use `name`.
pub fn allow_project(journal: &Journal, name: &str, project: &str) -> Result<(), String> {
    let mut all = read(journal);
    let found = all.iter_mut().find(|s| s.name == name).ok_or_else(|| format!("no key called {name}"))?;
    if !found.projects.iter().any(|p| p == project) {
        found.projects.push(project.to_owned());
    }
    keep(journal, &all)
}

/// A use of a kept key, as the journal records it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Used {
    pub name: String,
    #[serde(flatten)]
    pub by: SecretUse,
}

/// Record a use: the secret's last use, the list of every use, and — on a
/// ticket — the ticket's own record. Who, what, when; never the value.
pub fn record(journal: &Journal, name: &str, usage: SecretUse) -> Result<(), String> {
    let mut all = read(journal);
    if let Some(found) = all.iter_mut().find(|s| s.name == name) {
        found.last_used = Some(usage.clone());
        keep(journal, &all)?;
    }
    let mut uses = uses(journal);
    uses.push(Used { name: name.to_owned(), by: usage.clone() });
    if uses.len() > USES_KEPT {
        uses.drain(..uses.len() - USES_KEPT);
    }
    journal.set_secret_uses(&serde_json::to_string(&uses).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(ticket) = usage.ticket {
        let line = format!("{name} → {}", if usage.project.is_empty() { "(no project)" } else { usage.project.as_str() });
        let _ = journal.ticket_event(ticket, &usage.member, "secret_used", &line);
    }
    Ok(())
}

/// Every use kept, oldest first.
pub fn uses(journal: &Journal) -> Vec<Used> {
    journal.secret_uses().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// A question to the person about a first use, waiting: which card answers
/// it, what it grants.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Grant {
    /// The question set (card) that asks it.
    pub set: u64,
    pub name: String,
    pub project: String,
    pub member: String,
}

pub fn grants(journal: &Journal) -> Vec<Grant> {
    journal.secret_grants().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn keep_grants(journal: &Journal, all: &[Grant]) {
    if let Ok(json) = serde_json::to_string(all) {
        let _ = journal.set_secret_grants(&json);
    }
}

/// What the person picked on a first-use card: its options are, in order,
/// this project, any project, no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Here,
    Any,
    No,
}

pub fn answer_of(picked: &[usize]) -> Answer {
    match picked.first() {
        Some(0) => Answer::Here,
        Some(1) => Answer::Any,
        _ => Answer::No,
    }
}

/// The person's answer applied: `project` gets the key, or any does, or nothing changes.
pub fn apply_answer(journal: &Journal, grant: &Grant, answer: Answer) -> Result<(), String> {
    match answer {
        Answer::Here => allow_project(journal, &grant.name, &grant.project),
        Answer::Any => {
            let all = read(journal);
            let found = all.iter().find(|s| s.name == grant.name).ok_or_else(|| format!("no key called {}", grant.name))?;
            set_scope(journal, &grant.name, &found.projects, true)
        }
        Answer::No => Ok(()),
    }
}

/// A secret to keep, as the member gave it, checked.
pub struct Draft {
    pub name: String,
    pub service: String,
    pub kind: SecretKind,
    pub value: String,
    pub projects: Option<Vec<String>>,
    pub note: String,
}

/// Check a `secret_save` call's arguments.
pub fn check_save(args: &Value) -> Result<Draft, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let name = text("name");
    if !valid_name(&name) {
        return Err(format!("{name:?}: a name is letters, digits, `.`, `_` and `-`, up to 64 (`posthog-project-key`)."));
    }
    let service = text("service");
    if service.is_empty() || service.len() > 120 {
        return Err("Say in `service` which service or site it is for (`posthog`).".into());
    }
    let kind = SecretKind::parse(&text("kind")).ok_or_else(|| format!("`kind` is one of {}.", SecretKind::ALL.iter().map(|k| k.code()).collect::<Vec<_>>().join(", ")))?;
    // Only the ends are trimmed: a key's inside is its own.
    let value = args.get("value").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    if value.is_empty() {
        return Err("Give the key itself in `value`.".into());
    }
    if value.len() > MOST_BYTES {
        return Err(format!("A key is at most {} KB; this is a file, not a key.", MOST_BYTES / 1024));
    }
    let projects = args.get("projects").map(|_| crate::board::arg_strs(args, "projects"));
    Ok(Draft { name, service, kind, value, projects, note: text("note").chars().take(300).collect() })
}

/// What the member is told after saving, and nothing of the value.
pub fn saved_text(secret: &SquadSecret, replaced: bool) -> String {
    let where_ = if secret.any_project {
        "any project".to_owned()
    } else if secret.projects.is_empty() {
        "no project yet — the first use on one asks the person".to_owned()
    } else {
        secret.projects.iter().map(|p| shown_project(p)).collect::<Vec<_>>().join(", ")
    };
    format!(
        "{} {} ({}, for {}) in equipe's secret store; this squad's members can use it on: {where_}. Use it with `secret_use` \
(an environment variable from your next turn on) or `secret_write` (a git-ignored file in your worktree). The value is \
masked in transcripts, tickets, memory and hand-ins from now on; don't paste it anywhere.",
        if replaced { "Replaced" } else { "Saved" },
        secret.name,
        secret.kind.code(),
        secret.service,
    )
}

fn shown_project(project: &str) -> String {
    if project == NO_PROJECT { "no project".to_owned() } else { project.to_owned() }
}

/// The squad's keys as `secret_list` answers: facts, one per line.
pub fn list_text(all: &[SquadSecret]) -> String {
    if all.is_empty() {
        return "No keys are kept for this squad yet. When you obtain one, save it with `secret_save`.".into();
    }
    let when = |secs: u64| crate::squad_memory::date(secs);
    all.iter()
        .map(|s| {
            let scope = if s.any_project { "any project".to_owned() } else if s.projects.is_empty() { "no project yet".to_owned() } else { s.projects.iter().map(|p| shown_project(p)).collect::<Vec<_>>().join(", ") };
            let last = match &s.last_used {
                Some(u) => format!("last used {} by {}", when(u.at), u.member),
                None => "never used".to_owned(),
            };
            let note = if s.note.is_empty() { String::new() } else { format!(" — {}", s.note) };
            format!("- {} · {} · {} · projects: {scope} · saved {} by {} · {last}{note}", s.name, s.kind.code(), s.service, when(s.created_at), s.created_by)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Check an `as_env` name.
pub fn valid_env(name: &str) -> bool {
    crate::secrets::valid_key(name)
}

// ───────────────────────── masking ─────────────────────────

/// Every value kept, to cut out of text: by a key (`sq:<squad>/<name>`,
/// `env:<id>`), so a value that changes replaces its old one.
static WATCHED: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// The shortest value masked: anything shorter would cut ordinary words.
const SHORTEST: usize = 6;

/// What stands where a value was.
fn marker(key: &str) -> String {
    match key.strip_prefix("sq:").and_then(|k| k.split_once('/')) {
        Some((_, name)) => format!("[secret:{name}]"),
        None => "[secret]".to_owned(),
    }
}

/// Start masking `value` under `key`.
pub fn watch(key: &str, value: &str) {
    let value = value.trim();
    if value.len() < SHORTEST {
        unwatch(key);
        return;
    }
    WATCHED.write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key.to_owned(), value.to_owned());
}

pub fn unwatch(key: &str) {
    WATCHED.write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(key);
}

/// The key a squad secret's value is watched under.
pub fn watch_key(squad: &str, name: &str) -> String {
    format!("sq:{squad}/{name}")
}

/// The value watched under `key`, for the daemon to hand to a command.
pub fn watched(key: &str) -> Option<String> {
    WATCHED.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(key).cloned()
}

/// The length of the longest form of a value kept (0: none kept): how much
/// of a streaming answer's end is held back so a value split across
/// fragments is still cut out whole.
pub fn longest() -> usize {
    let watched = WATCHED.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    watched.values().map(|v| v.len().max(serde_json::to_string(v).map_or(0, |s| s.len().saturating_sub(2)))).max().unwrap_or(0)
}

/// `text` with every value kept cut out, verbatim: the value itself, its
/// JSON-escaped form, and each line of a multi-line one. Borrowed when there
/// was nothing to cut.
pub fn mask(text: &str) -> Cow<'_, str> {
    let watched = WATCHED.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    if watched.is_empty() || text.len() < SHORTEST {
        return Cow::Borrowed(text);
    }
    let mut out: Option<String> = None;
    for (key, value) in watched.iter() {
        let here = out.as_deref().unwrap_or(text);
        let marker = marker(key);
        let mut forms: Vec<String> = vec![value.clone()];
        let escaped = serde_json::to_string(value).map(|s| s[1..s.len() - 1].to_owned()).unwrap_or_default();
        if escaped != *value && !escaped.is_empty() {
            forms.push(escaped);
        }
        if value.contains('\n') {
            forms.extend(value.lines().map(str::trim).filter(|l| l.len() >= 12).map(str::to_owned));
        }
        // Longest first: a line of a key mustn't leave the rest of the key behind.
        forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
        if forms.iter().any(|f| here.contains(f.as_str())) {
            let mut next = here.to_owned();
            for form in &forms {
                next = next.replace(form.as_str(), &marker);
            }
            out = Some(next);
        }
    }
    match out {
        Some(text) => Cow::Owned(text),
        None => Cow::Borrowed(text),
    }
}

/// [`mask`] on every string inside a JSON value; whether anything changed.
pub fn mask_json(value: &mut Value) -> bool {
    match value {
        Value::String(s) => match mask(s) {
            Cow::Owned(masked) => {
                *s = masked;
                true
            }
            Cow::Borrowed(_) => false,
        },
        Value::Array(items) => items.iter_mut().fold(false, |any, v| mask_json(v) | any),
        Value::Object(map) => map.values_mut().fold(false, |any, v| mask_json(v) | any),
        _ => false,
    }
}

/// What stands where something that looks like a key was cut out.
pub const KEY_MASK: &str = "[key redacted]";

// ───────────────────────── what looks like a key ─────────────────────────

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn value_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_-./+=".contains(c)
}

/// A value that reads as made-up random text, not a word or a placeholder.
fn random_looking(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if ["xxxx", "your", "example", "changeme", "placeholder", "redacted", "....", "****", "secret_here", "dummy", "sample"].iter().any(|p| lower.contains(p)) {
        return false;
    }
    let digits = value.chars().filter(char::is_ascii_digit).count();
    let letters = value.chars().filter(char::is_ascii_alphabetic).count();
    digits >= 2 && letters >= 4
}

/// Where in `text` something looks like a key a member obtained and didn't
/// save: a well-known key's prefix, a password inside a connection string's
/// address, the value of a `…KEY=`, `…TOKEN:` or `Bearer` line. Ranges in
/// bytes, sorted and apart.
pub fn keys_in(text: &str) -> Vec<Range<usize>> {
    let mut found: Vec<Range<usize>> = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    // Runs of word characters: a well-known key's prefix.
    let mut i = 0;
    while i < chars.len() {
        if !word_char(chars[i].1) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && word_char(chars[i].1) {
            i += 1;
        }
        let (from, to) = (chars[start].0, chars.get(i).map_or(text.len(), |c| c.0));
        let run = &text[from..to];
        if run.len() >= 20 && crate::signals::KEY_PREFIXES.iter().any(|p| run.starts_with(p)) && run.chars().any(|c| c.is_ascii_digit()) && random_looking(run) {
            found.push(from..to);
        }
    }
    // `scheme://user:password@host`: the password.
    let mut at = 0;
    while let Some(p) = text[at..].find("://") {
        let auth_from = at + p + 3;
        let auth_to = text[auth_from..].find(|c: char| c.is_whitespace() || "/?#\"'<>".contains(c)).map_or(text.len(), |e| auth_from + e);
        let authority = &text[auth_from..auth_to];
        if let Some(userinfo_end) = authority.rfind('@')
            && let Some(colon) = authority[..userinfo_end].find(':')
        {
            let password = &authority[colon + 1..userinfo_end];
            let plain = password.is_empty() || password.starts_with(['$', '{', '<', '*', '[']) || password.len() < 6 || ["password", "pass", "secret", "xxx"].iter().any(|p| password.eq_ignore_ascii_case(p));
            if !plain {
                found.push(auth_from + colon + 1..auth_from + userinfo_end);
            }
        }
        at = auth_to.max(auth_from);
    }
    // `NAME_KEY=value`, `"token": "value"`, `Authorization: Bearer value`.
    for (n, &(pos, c)) in chars.iter().enumerate() {
        if c != '=' && c != ':' && c != ' ' {
            continue;
        }
        // The word before, past quotes and spaces.
        let mut b = n;
        while b > 0 && (chars[b - 1].1 == '"' || chars[b - 1].1 == '\'' || chars[b - 1].1 == ' ') {
            b -= 1;
        }
        let end = b;
        while b > 0 && (word_char(chars[b - 1].1) || chars[b - 1].1 == '.') {
            b -= 1;
        }
        if b == end {
            continue;
        }
        let name = text[chars[b].0..chars.get(end).map_or(text.len(), |c| c.0)].to_ascii_lowercase();
        let bearer = c == ' ' && name == "bearer";
        let named = c != ' ' && ["key", "token", "secret", "password", "passwd", "credential"].iter().any(|w| name.contains(w));
        if !bearer && !named {
            continue;
        }
        let mut v = n + 1;
        while v < chars.len() && (chars[v].1 == ' ' || chars[v].1 == '"' || chars[v].1 == '\'') {
            v += 1;
        }
        let vstart = v;
        while v < chars.len() && value_char(chars[v].1) {
            v += 1;
        }
        let (from, to) = (chars.get(vstart).map_or(text.len(), |c| c.0), chars.get(v).map_or(text.len(), |c| c.0));
        let value = &text[from..to];
        if value.len() >= 20 && random_looking(value) && pos < from {
            found.push(from..to);
        }
    }
    found.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in found {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// `text` with what looks like a key cut out, and how many were.
pub fn scrub(text: &str) -> (String, usize) {
    let found = keys_in(text);
    if found.is_empty() {
        return (text.to_owned(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for r in &found {
        out.push_str(&text[last..r.start]);
        out.push_str(KEY_MASK);
        last = r.end;
    }
    out.push_str(&text[last..]);
    (out, found.len())
}

/// [`scrub`] on every string inside a JSON value: how many keys were cut out.
pub fn scrub_json(value: &mut Value) -> usize {
    match value {
        Value::String(s) => {
            let (clean, n) = scrub(s);
            if n > 0 {
                *s = clean;
            }
            n
        }
        Value::Array(items) => items.iter_mut().map(scrub_json).sum(),
        Value::Object(map) => map.values_mut().map(scrub_json).sum(),
        _ => 0,
    }
}

/// What a member is told when something that looked like a key was cut out
/// of what it said or what a command printed.
pub const SAVE_NUDGE: &str = "equipe cut something that looks like a key or password out of what you wrote or a command printed. If it is a key you obtained, \
keep it with `secret_save` ({name, service, kind, value}) so it can be used later, and don't paste it into code, tickets, memory or chat.";

// ───────────────────────── writing into the worktree ─────────────────────────

/// How a secret is written into a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Env,
    Json,
    Raw,
}

impl Format {
    pub fn parse(word: Option<&str>) -> Result<Self, String> {
        match word.map(|w| w.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("env") => Ok(Self::Env),
            Some("json") => Ok(Self::Json),
            Some("raw") => Ok(Self::Raw),
            Some(other) => Err(format!("`format` is env, json or raw, not {other:?}.")),
        }
    }
}

/// Whether git ignores `rel` in `worktree`: `Some(true)` it does, `Some(false)` it
/// doesn't, `None` this isn't a git working tree.
fn git_ignores(worktree: &Path, rel: &str) -> Option<bool> {
    let status = git(worktree, &["check-ignore", "-q", "--", rel])?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn git_tracks(worktree: &Path, rel: &str) -> Option<bool> {
    let status = git(worktree, &["ls-files", "--error-unmatch", "--", rel])?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn git(worktree: &Path, args: &[&str]) -> Option<std::process::ExitStatus> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
}

/// Where in `worktree` `rel` may be written: inside it, not through a link,
/// not in `.git`, not a folder — and git-ignored and untracked, or refused,
/// saying how to fix it.
pub fn place(worktree: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    if rel.is_empty() {
        return Err("Say in `path` which file, relative to your worktree: `.env.local`.".into());
    }
    let path = Path::new(rel);
    if path.is_absolute() || path.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("{rel}: give the file relative to your worktree, without `..` (`.env.local`)."));
    }
    if matches!(path.components().next(), Some(Component::Normal(first)) if first == ".git") {
        return Err(format!("{rel}: never inside .git."));
    }
    let root = worktree.canonicalize().map_err(|_| "Your worktree can't be read.".to_owned())?;
    let target = root.join(path);
    // The deepest folder there is must still be inside the worktree once links are followed.
    let mut probe = target.parent();
    while let Some(dir) = probe {
        if dir.exists() {
            let real = dir.canonicalize().map_err(|e| e.to_string())?;
            if !real.starts_with(&root) {
                return Err(format!("{rel}: that goes outside your worktree through a link."));
            }
            break;
        }
        probe = dir.parent();
    }
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(format!("{rel}: that is a link; name the file itself.")),
        Ok(meta) if meta.is_dir() => return Err(format!("{rel}: that is a folder; name the file.")),
        _ => {}
    }
    match (git_ignores(&root, rel), git_tracks(&root, rel)) {
        (None, _) | (_, None) => return Err(format!("{rel}: your worktree isn't a git working tree, so equipe can't tell the file is kept out of commits; secrets are only written into git-ignored files.")),
        (_, Some(true)) => return Err(format!("{rel}: git tracks that file, so a key written there would be committed. Use a file that is git-ignored and untracked.")),
        (Some(false), _) => return Err(format!("{rel}: that file isn't git-ignored, so a key written there could be committed. Add it to .gitignore (and don't commit that file), then ask again.")),
        (Some(true), Some(false)) => {}
    }
    Ok(target)
}

/// Write the file text for the person alone, in one move.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let draft = path.with_extension("equipe-draft");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&draft).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&draft, path).map_err(|e| e.to_string())
}

/// Put `value` into `path` (already [`place`]d) in `format`, as `key`.
pub fn write_into(path: &Path, format: Format, key: &str, value: &str) -> Result<(), String> {
    match format {
        Format::Env => {
            if !valid_env(key) {
                return Err(format!("{key:?}: an environment variable is uppercase letters, digits and _, starting with a letter (POSTHOG_KEY)."));
            }
            let field = crate::protocol::secrets::SecretField { key: key.to_owned(), label: key.to_owned(), secret: true };
            crate::secrets::write(path, &[field], &[crate::protocol::secrets::SecretValue(value.to_owned())])
        }
        Format::Json => {
            let mut object = match std::fs::read_to_string(path) {
                Ok(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(&text).map_err(|_| "that file isn't JSON".to_owned())?,
                _ => json!({}),
            };
            let map = object.as_object_mut().ok_or("that file's JSON isn't an object")?;
            map.insert(key.to_owned(), Value::String(value.to_owned()));
            write_private(path, &(serde_json::to_string_pretty(&object).map_err(|e| e.to_string())? + "\n"))
        }
        Format::Raw => write_private(path, value),
    }
}

/// The variable or field a secret is written as when the member named none.
pub fn default_key(name: &str, format: Format) -> String {
    match format {
        Format::Env => name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect::<String>().trim_start_matches(|c: char| !c.is_ascii_uppercase()).to_owned(),
        _ => name.to_owned(),
    }
}

/// A use as the journal wants it.
pub fn usage(member: &str, ticket: Option<TicketId>, project: &str, how: &str, at: u64) -> SecretUse {
    SecretUse { at, member: member.to_owned(), ticket, project: project.to_owned(), how: how.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(name: &str, projects: &[&str], any: bool) -> SquadSecret {
        SquadSecret {
            squad: "acme".into(),
            name: name.into(),
            kind: SecretKind::ApiKey,
            service: "posthog".into(),
            projects: projects.iter().map(|p| (*p).to_owned()).collect(),
            any_project: any,
            created_by: "backend".into(),
            created_ticket: Some(3),
            created_at: 1_791_849_600,
            note: String::new(),
            last_used: None,
        }
    }

    /// A number no other test of this process has had.
    fn unique() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn journal() -> (Journal, PathBuf) {
        let dir = std::env::temp_dir().join(format!("equipe-squad-secrets-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&dir).unwrap();
        (Journal::open(&dir.join("squad.db")).unwrap(), dir)
    }

    #[test]
    fn names_are_plain() {
        assert!(valid_name("posthog-project-key") && valid_name("A1.b_c"));
        let long = "x".repeat(65);
        for bad in ["", "-x", "a b", "a/b", long.as_str()] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert_eq!(account("acme", "k"), "acme/k");
    }

    #[test]
    fn a_save_call_is_checked() {
        let ok = check_save(&json!({ "name": "ph", "service": "posthog", "kind": "api-key", "value": "  phc_abc123  ", "projects": ["web"], "note": "n" })).unwrap();
        assert_eq!((ok.value.as_str(), ok.kind, ok.projects), ("phc_abc123", SecretKind::ApiKey, Some(vec!["web".to_owned()])));
        for bad in [
            json!({ "name": "p h", "service": "s", "kind": "token", "value": "v" }),
            json!({ "name": "ph", "service": "", "kind": "token", "value": "v" }),
            json!({ "name": "ph", "service": "s", "kind": "magic", "value": "v" }),
            json!({ "name": "ph", "service": "s", "kind": "token", "value": "  " }),
        ] {
            assert!(check_save(&bad).is_err(), "{bad}");
        }
        assert!(check_save(&json!({ "name": "ph", "service": "s", "kind": "token", "value": "x".repeat(MOST_BYTES + 1) })).is_err());
        assert_eq!(check_save(&json!({ "name": "ph", "service": "s", "kind": "token", "value": "v" })).unwrap().projects, None);
    }

    #[test]
    fn facts_are_kept_in_the_journal_and_the_value_is_nowhere_in_them() {
        let (journal, dir) = journal();
        put(&journal, secret("b", &["web"], false)).unwrap();
        put(&journal, secret("a", &[], true)).unwrap();
        let all = read(&journal);
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        // Putting a name again replaces it.
        put(&journal, SquadSecret { service: "vercel".into(), ..secret("b", &["web"], false) }).unwrap();
        assert_eq!(read(&journal).len(), 2);
        assert_eq!(read(&journal)[1].service, "vercel");
        assert!(remove(&journal, "a").unwrap() && !remove(&journal, "a").unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_project_the_person_didnt_allow_asks_once_and_any_never_asks() {
        let (journal, dir) = journal();
        put(&journal, secret("k", &["web"], false)).unwrap();
        let all = read(&journal);
        assert_eq!(access(&all[0], "web"), Access::Allowed);
        assert_eq!(access(&all[0], "api"), Access::Ask);
        assert_eq!(access(&all[0], NO_PROJECT), Access::Ask);
        let grant = Grant { set: 7, name: "k".into(), project: "api".into(), member: "backend".into() };
        apply_answer(&journal, &grant, Answer::No).unwrap();
        assert_eq!(access(&read(&journal)[0], "api"), Access::Ask);
        apply_answer(&journal, &grant, Answer::Here).unwrap();
        assert_eq!(access(&read(&journal)[0], "api"), Access::Allowed);
        assert_eq!(access(&read(&journal)[0], "mobile"), Access::Ask);
        apply_answer(&journal, &grant, Answer::Any).unwrap();
        assert_eq!(access(&read(&journal)[0], "mobile"), Access::Allowed);
        assert_eq!((answer_of(&[0]), answer_of(&[1]), answer_of(&[2]), answer_of(&[])), (Answer::Here, Answer::Any, Answer::No, Answer::No));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_use_is_recorded_with_who_ticket_and_when() {
        let (journal, dir) = journal();
        put(&journal, secret("k", &["web"], false)).unwrap();
        record(&journal, "k", usage("frontend", None, "web", "env", 100)).unwrap();
        record(&journal, "k", usage("backend", None, "web", "file", 200)).unwrap();
        let last = read(&journal)[0].last_used.clone().unwrap();
        assert_eq!((last.member.as_str(), last.at, last.how.as_str()), ("backend", 200, "file"));
        let all = uses(&journal);
        assert_eq!(all.iter().map(|u| (u.name.as_str(), u.by.member.as_str())).collect::<Vec<_>>(), [("k", "frontend"), ("k", "backend")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_stored_value_is_cut_out_verbatim_in_any_text_and_its_json_form() {
        watch("sq:acme/tests-ph", "phc_StoredValue987654321");
        watch("sq:acme/tests-pem", "-----BEGIN KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n-----END KEY-----");
        let said = "ran with phc_StoredValue987654321 and {\"k\":\"phc_StoredValue987654321\"}";
        assert_eq!(mask(said), "ran with [secret:tests-ph] and {\"k\":\"[secret:tests-ph]\"}");
        assert!(matches!(mask("nothing to see here at all"), Cow::Borrowed(_)));
        // A multi-line key is cut by line too: a log may print one line of it.
        assert_eq!(mask("line MIIEvQIBADANBgkqhkiG9w0BAQEFAASC end"), "line [secret:tests-pem] end");
        let mut value = json!({ "a": ["x phc_StoredValue987654321"], "n": 1 });
        assert!(mask_json(&mut value));
        assert_eq!(value["a"][0], "x [secret:tests-ph]");
        // Too short to cut without cutting words.
        watch("sq:acme/tests-short", "abc");
        assert_eq!(mask("abc abc"), "abc abc");
        unwatch("sq:acme/tests-ph");
        assert_eq!(mask("phc_StoredValue987654321"), "phc_StoredValue987654321", "an unwatched value is no longer cut");
        unwatch("sq:acme/tests-pem");
    }

    #[test]
    fn what_looks_like_a_key_is_spotted_and_ordinary_text_is_not() {
        let text = "got key phc_aB3dE5gH7jK9mN1pQ3sT5vW7yZ0 and DATABASE_URL=postgres://app:Zq8vK2mX9wLp@db.internal/main plus \
Authorization: Bearer eyJhbGciOiJIUzI1NiJ9abcdef123456 and \"api_key\": \"AbCd1234EfGh5678IjKl9012\"";
        let (clean, n) = scrub(text);
        assert_eq!(n, 4, "{clean}");
        for gone in ["phc_aB3dE5gH7jK9mN1pQ3sT5vW7yZ0", "Zq8vK2mX9wLp", "eyJhbGciOiJIUzI1NiJ9abcdef123456", "AbCd1234EfGh5678IjKl9012"] {
            assert!(!clean.contains(gone), "{gone} in {clean}");
        }
        assert!(clean.contains("postgres://app:[key redacted]@db.internal/main"), "{clean}");
        for fine in [
            "commit 3f786850e387550fdab836ed7e6dc881de23001b merged",
            "API_KEY=your-api-key-here-1234567890",
            "postgres://user:password@localhost/db",
            "token: ${GITHUB_TOKEN_FOR_CI_RUNS_1234}",
            "see the sk-learn docs",
            "export POSTHOG_KEY=$POSTHOG_KEY",
            "the key phrase is long-enough-but-plain-words",
        ] {
            assert_eq!(scrub(fine), (fine.to_owned(), 0), "{fine}");
        }
        let mut value = json!({ "summary": "used phc_aB3dE5gH7jK9mN1pQ3sT5vW7yZ0 here" });
        assert_eq!(scrub_json(&mut value), 1);
        assert_eq!(value["summary"], "used [key redacted] here");
    }

    fn repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-squad-secrets-repo-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).env_remove("GIT_DIR").output().unwrap().status.success(), "{args:?}");
        run(&["init", "-q"]);
        std::fs::write(dir.join(".gitignore"), ".env.local\nsecrets/\n").unwrap();
        std::fs::write(dir.join("tracked.env"), "A=1\n").unwrap();
        run(&["add", "tracked.env"]);
        dir.canonicalize().unwrap()
    }

    #[test]
    fn only_a_git_ignored_untracked_file_inside_the_worktree_may_hold_a_key() {
        let dir = repo();
        assert!(place(&dir, ".env.local").is_ok());
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        assert!(place(&dir, "./secrets/ph.json").is_ok(), "a folder git ignores");
        let refused = |rel: &str| place(&dir, rel).unwrap_err();
        assert!(refused("README.md").contains("isn't git-ignored"));
        assert!(refused("tracked.env").contains("git tracks"), "tracked");
        assert!(refused("../out.env").contains("without `..`"));
        assert!(refused("/etc/passwd").contains("relative"));
        assert!(refused(".git/config").contains(".git"));
        assert!(refused("").contains("Say in `path`"));
        std::fs::create_dir_all(dir.join("secrets/inner")).unwrap();
        assert!(refused("secrets/inner").contains("folder"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", dir.join("secrets/out")).unwrap();
            assert!(refused("secrets/out/x.env").contains("outside your worktree") || refused("secrets/out/x.env").contains("link"));
            std::os::unix::fs::symlink(dir.join("tracked.env"), dir.join(".env.local")).unwrap();
            assert!(refused(".env.local").contains("link"));
        }
        // Not a repository at all: refused, said why.
        let plain = std::env::temp_dir().join(format!("equipe-squad-secrets-plain-{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        let plain = plain.canonicalize().unwrap();
        assert!(place(&plain, ".env.local").unwrap_err().contains("isn't a git working tree"));
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(plain).unwrap();
    }

    #[test]
    fn a_key_is_written_for_the_person_alone_in_each_format() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = repo();
        let env = place(&dir, ".env.local").unwrap();
        std::fs::write(&env, "OTHER=\"kept\"\n").unwrap();
        write_into(&env, Format::Env, "POSTHOG_KEY", "phc_x\"y$z").unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "OTHER=\"kept\"\nPOSTHOG_KEY=\"phc_x\\\"y\\$z\"\n");
        assert_eq!(std::fs::metadata(&env).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(write_into(&env, Format::Env, "lower", "v").is_err());
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        let json_file = place(&dir, "secrets/ph.json").unwrap();
        write_into(&json_file, Format::Json, "apiKey", "v1").unwrap();
        write_into(&json_file, Format::Json, "other", "v2").unwrap();
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&json_file).unwrap()).unwrap(), json!({ "apiKey": "v1", "other": "v2" }));
        assert_eq!(std::fs::metadata(&json_file).unwrap().permissions().mode() & 0o777, 0o600);
        let raw = place(&dir, "secrets/token.txt").unwrap();
        write_into(&raw, Format::Raw, "", "tok").unwrap();
        assert_eq!(std::fs::read_to_string(&raw).unwrap(), "tok");
        assert_eq!(default_key("posthog-project.key", Format::Env), "POSTHOG_PROJECT_KEY");
        assert_eq!(default_key("9lives", Format::Env), "LIVES");
        assert_eq!(Format::parse(None).unwrap(), Format::Env);
        assert!(Format::parse(Some("yaml")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_tools_never_ask_for_or_return_a_value_except_to_save() {
        let all = tools();
        assert_eq!(all.iter().map(|t| t["name"].as_str().unwrap()).collect::<Vec<_>>(), [SAVE, LIST, USE, WRITE]);
        assert!(is_tool("mcp__equipe__secret_save", SAVE) && !is_tool("mcp__equipe__secret_saved", SAVE));
        assert!(carries_value("mcp__equipe__secret_save") && !carries_value(USE));
        for tool in &all[1..] {
            assert!(tool["inputSchema"]["properties"].get("value").is_none(), "{tool}");
        }
    }
}
