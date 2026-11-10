//! Reading what the engines on this machine used from their own transcripts:
//! Claude Code's `~/.claude/projects/**/*.jsonl` (each request's `usage`,
//! once per message id) and the Codex CLI's `~/.codex/sessions/**/rollout-*.jsonl`
//! (each request's `token_usage_record`, or the `token_count` totals where a
//! rollout has none). Every Claude Code and Codex session counts against the
//! person's plan — equipe's members' and the person's own alike — so this is
//! where the share outside equipe is seen.
//!
//! Only the numbers are read: a line is parsed into the few fields it needs
//! (the rest, the conversation itself, is skipped as it is parsed and never
//! kept), and only lines that can hold usage are parsed at all. Nothing else
//! of the engines' folders is opened — never a credential file — and nothing
//! leaves the machine. Each file is read on from where it was last left
//! ([`Ledger::cursor`]), so a file is never read twice; a whole first pass
//! reads only the files written in the last [`ledger::KEPT`].

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::ledger::{self, Key, Ledger, Used};
use crate::protocol::plan_use::Tokens;

/// One request's use, as a transcript line says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub at: i64,
    pub model: String,
    pub session: String,
    pub cwd: String,
    pub origin: String,
    pub used: Used,
    /// Counted once by this, however many lines repeat it; its output may
    /// grow from one line to the next.
    pub key: Option<String>,
}

// ── Claude Code ──

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    request_id: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheCreation {
    ephemeral_1h_input_tokens: u64,
}

/// A Claude Code transcript line's request, if it is one with usage.
pub fn claude_line(line: &str) -> Option<Sample> {
    if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
        return None;
    }
    let d: ClaudeLine = serde_json::from_str(line).ok()?;
    if d.kind.as_deref() != Some("assistant") {
        return None;
    }
    let message = d.message?;
    let usage = message.usage?;
    let model = message.model.unwrap_or_default();
    // Claude Code's own stand-ins for an answer cost nothing.
    if model.starts_with('<') {
        return None;
    }
    let tokens = Tokens { input: usage.input_tokens, output: usage.output_tokens, cache_write: usage.cache_creation_input_tokens, cache_read: usage.cache_read_input_tokens };
    if tokens.total() == 0 {
        return None;
    }
    let long = usage.cache_creation.map_or(0, |c| c.ephemeral_1h_input_tokens).min(tokens.cache_write);
    Some(Sample {
        at: d.timestamp.as_deref().and_then(iso_secs)?,
        model,
        session: d.session_id.unwrap_or_default(),
        cwd: d.cwd.unwrap_or_default(),
        origin: String::new(),
        used: Used { tokens, long, calls: 1 },
        key: message.id.or(d.request_id).map(|id| format!("claude:{id}")),
    })
}

// ── Codex ──

/// What a rollout's earlier lines said that its later ones go by.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexFile {
    pub session: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub origin: Option<String>,
    /// It has a record of each request: its totals are not counted then.
    #[serde(default)]
    pub records: bool,
    /// The totals as of its last `token_count`: input, cached, written, output.
    #[serde(default)]
    pub total: Option<[u64; 4]>,
}

#[derive(Deserialize)]
struct CodexLine {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    payload: Option<CodexPayload>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    originator: Option<String>,
    model: Option<String>,
    response_id: Option<String>,
    usage: Option<CodexUsage>,
    info: Option<CodexInfo>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodexInfo {
    total_token_usage: Option<CodexUsage>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct CodexUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
}

impl CodexUsage {
    fn array(self) -> [u64; 4] {
        [self.input_tokens, self.cached_input_tokens, self.cache_write_input_tokens, self.output_tokens]
    }
}

/// Codex counts cached input in its input: what was neither read nor written.
fn codex_tokens([input, cached, written, output]: [u64; 4]) -> Tokens {
    Tokens { input: input.saturating_sub(cached).saturating_sub(written), output, cache_write: written, cache_read: cached }
}

/// A Codex rollout line's request, if it is one; `file` carries what the
/// lines before it said.
pub fn codex_line(line: &str, file: &mut CodexFile) -> Option<Sample> {
    let wanted = ["\"session_meta\"", "\"turn_context\"", "\"token_usage_record\"", "\"token_count\""];
    if !wanted.iter().any(|w| line.contains(w)) {
        return None;
    }
    let d: CodexLine = serde_json::from_str(line).ok()?;
    let p = d.payload.unwrap_or_default();
    let sample = |file: &CodexFile, tokens: Tokens, key: Option<String>| {
        Some(Sample {
            at: d.timestamp.as_deref().and_then(iso_secs)?,
            model: file.model.clone().unwrap_or_default(),
            session: file.session.clone().unwrap_or_default(),
            cwd: file.cwd.clone().unwrap_or_default(),
            origin: file.origin.clone().unwrap_or_default(),
            used: Used { tokens, long: 0, calls: 1 },
            key,
        })
    };
    match (d.kind.as_str(), p.kind.as_deref()) {
        // The first is the file's own; a subagent's file goes on with its parent's.
        ("session_meta", _) => {
            if file.session.is_none() {
                file.session = p.session_id.or(p.id);
                file.cwd = p.cwd;
                file.origin = p.originator;
            }
            None
        }
        ("turn_context", _) => {
            if p.model.is_some() {
                file.model = p.model;
            }
            if p.cwd.is_some() {
                file.cwd = p.cwd;
            }
            None
        }
        ("token_usage_record", _) => {
            file.records = true;
            let tokens = codex_tokens(p.usage?.array());
            if tokens.total() == 0 {
                return None;
            }
            sample(file, tokens, p.response_id.map(|id| format!("codex:{id}")))
        }
        ("event_msg", Some("token_count")) if !file.records => {
            let now = p.info?.total_token_usage?.array();
            let before = file.total.unwrap_or_default();
            file.total = Some(now);
            // Said again unchanged (a limits update): nothing new.
            let delta: Vec<u64> = now.iter().zip(before).map(|(n, b)| n.saturating_sub(b)).collect();
            let tokens = codex_tokens([delta[0], delta[1], delta[2], delta[3]]);
            if tokens.total() == 0 {
                return None;
            }
            sample(file, tokens, None)
        }
        _ => None,
    }
}

/// `2026-11-02T15:59:09.215Z` (or with an offset) in Unix seconds.
pub fn iso_secs(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // The zone, after any fraction of a second.
    let rest = &s[19..];
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.as_bytes().first() {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            let (h, m) = (digits.get(0..2)?.parse::<i64>().ok()?, digits.get(2..4).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0));
            let secs = h * 3600 + m * 60;
            if *sign == b'+' { secs } else { -secs }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

// ── the files ──

/// Where the engines keep their transcripts on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    /// Claude Code's projects folder.
    pub claude: Option<PathBuf>,
    /// The Codex CLI's sessions folder.
    pub codex: Option<PathBuf>,
}

impl Roots {
    /// As the engines find them: `$CLAUDE_CONFIG_DIR/projects` else
    /// `~/.claude/projects`; `$CODEX_HOME/sessions` else `~/.codex/sessions`.
    pub fn of_this_machine() -> Self {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let codex = std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".codex")));
        Roots { claude: crate::quality::claude_projects(), codex: codex.map(|c| c.join("sessions")) }
    }
}

/// Which engine a transcript is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Claude,
    Codex,
}

/// The transcripts under `roots`: Claude Code's every `.jsonl`, Codex's
/// `rollout-*.jsonl` only.
fn files(roots: &Roots) -> Vec<(Kind, PathBuf)> {
    fn walk(dir: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() && depth < 6 {
                walk(&path, depth + 1, keep, out);
            } else if kind.is_file() && keep(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = &roots.claude {
        let mut found = Vec::new();
        walk(root, 0, &|p| p.extension().is_some_and(|x| x == "jsonl"), &mut found);
        out.extend(found.into_iter().map(|p| (Kind::Claude, p)));
    }
    if let Some(root) = &roots.codex {
        let mut found = Vec::new();
        let rollout = |p: &Path| p.extension().is_some_and(|x| x == "jsonl") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rollout-"));
        walk(root, 0, &rollout, &mut found);
        out.extend(found.into_iter().map(|p| (Kind::Codex, p)));
    }
    out
}

/// What a pass over the transcripts did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    pub files: usize,
    pub bytes: u64,
    pub requests: u64,
    /// It read everything there was (not stopped by its time).
    pub complete: bool,
}

/// One pass at a time, whoever asks: a file read twice at once would count twice.
static PASS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Read on in every transcript under `roots` from where it was left, into
/// `ledger`. Stops after `budget`, if given, with the rest for the next pass.
/// Blocking: off the loop.
pub fn scan(ledger: &Ledger, roots: &Roots, now: i64, budget: Option<Duration>) -> Scanned {
    let _pass = PASS.lock().unwrap_or_else(|e| e.into_inner());
    let started = Instant::now();
    let mut scanned = Scanned { complete: true, ..Default::default() };
    let mut all = files(roots);
    // The newest first, so a pass cut short has the latest hours.
    all.sort_by_cached_key(|(_, p)| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    for (kind, path) in all {
        if budget.is_some_and(|b| started.elapsed() > b) {
            scanned.complete = false;
            break;
        }
        match read_file(ledger, kind, &path, now) {
            Ok(Some((bytes, requests))) => {
                scanned.files += 1;
                scanned.bytes += bytes;
                scanned.requests += requests;
            }
            Ok(None) => {}
            Err(e) => crate::daemon_log!("equipe: couldn't read the usage in {}: {e}", path.display()),
        }
    }
    let _ = ledger.prune(now);
    if scanned.complete {
        let _ = ledger.set_meta("read_once", "1");
    }
    scanned
}

/// Whether a whole pass has been made: until then, what is known is short.
pub fn read_once(ledger: &Ledger) -> bool {
    ledger.meta("read_once").is_some()
}

/// Read one file on from its cursor: the bytes read and requests counted,
/// or `None` when there was nothing new.
fn read_file(ledger: &Ledger, kind: Kind, path: &Path, now: i64) -> std::io::Result<Option<(u64, u64)>> {
    let meta = std::fs::metadata(path)?;
    let len = meta.len();
    let key = path.to_string_lossy().into_owned();
    let cursor = ledger.cursor(&key).map_err(std::io::Error::other)?;
    let modified = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(now, |d| d.as_secs() as i64);
    let (mut offset, state) = match cursor {
        // Not written to in what is kept: never read.
        None if modified < now - ledger::KEPT => {
            ledger.set_cursor(&key, len, None).map_err(std::io::Error::other)?;
            return Ok(None);
        }
        None => (0, None),
        Some((offset, _)) if offset == len => return Ok(None),
        // Shorter than when it was read: written anew, read anew.
        Some((offset, _)) if offset > len => (0, None),
        Some((offset, state)) => (offset, state),
    };
    let mut codex: CodexFile = state.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut line = Vec::new();
    let mut samples = Vec::new();
    let start = offset;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        // A line still being written waits for the next pass.
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        offset += n as u64;
        let Ok(text) = std::str::from_utf8(&line) else { continue };
        let sample = match kind {
            Kind::Claude => claude_line(text),
            Kind::Codex => codex_line(text, &mut codex),
        };
        if let Some(s) = sample.filter(|s| s.at >= now - ledger::KEPT) {
            samples.push(s);
        }
    }
    let engine = if kind == Kind::Claude { "claude" } else { "codex" };
    let requests = samples.len() as u64;
    ledger.begin().map_err(std::io::Error::other)?;
    let added = (|| -> rusqlite::Result<()> {
        let mut hours: HashMap<Key, Used> = HashMap::new();
        for s in samples {
            let used = match &s.key {
                None => s.used,
                Some(k) => match ledger.seen(k, s.used.tokens.output, s.at)? {
                    None => s.used,
                    // Written again, its output further along: only what is new.
                    Some(before) if s.used.tokens.output > before => Used { tokens: Tokens { output: s.used.tokens.output - before, ..Default::default() }, long: 0, calls: 0 },
                    Some(_) => continue,
                },
            };
            let key = Key { hour: s.at - s.at.rem_euclid(3600), engine: engine.into(), model: s.model, session: s.session, cwd: s.cwd, origin: s.origin };
            hours.entry(key).or_default().add(used);
        }
        for (key, used) in &hours {
            ledger.add(key, *used)?;
        }
        let state = (kind == Kind::Codex).then(|| serde_json::to_string(&codex).unwrap_or_default());
        ledger.set_cursor(&key, offset, state.as_deref())
    })();
    match added {
        Ok(()) => ledger.commit().map_err(std::io::Error::other)?,
        Err(e) => {
            let _ = ledger.conn().execute_batch("ROLLBACK");
            return Err(std::io::Error::other(e));
        }
    }
    Ok(Some((offset - start, requests)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = r#"{"type":"assistant","timestamp":"2026-11-02T15:59:09.215Z","sessionId":"s1","cwd":"/u/code/site","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"text","text":"a secret the person wrote"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":25969,"cache_read_input_tokens":11347,"output_tokens":272,"cache_creation":{"ephemeral_1h_input_tokens":25969,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn a_claude_request_is_read_from_its_usage_alone() {
        let s = claude_line(CLAUDE).unwrap();
        assert_eq!(s.at, 1_793_635_149);
        assert_eq!((s.model.as_str(), s.session.as_str(), s.cwd.as_str()), ("claude-opus-5-5", "s1", "/u/code/site"));
        assert_eq!(s.used.tokens, Tokens { input: 2, output: 272, cache_write: 25969, cache_read: 11347 });
        assert_eq!((s.used.long, s.used.calls), (25969, 1));
        assert_eq!(s.key.as_deref(), Some("claude:msg_1"));
        let said = format!("{s:?}");
        assert!(!said.contains("secret"), "the words are never kept: {said}");
    }

    #[test]
    fn lines_that_are_no_request_are_passed_over() {
        assert_eq!(claude_line(r#"{"type":"user","message":{"role":"user","content":"what is \"usage\"? assistant"}}"#), None);
        assert_eq!(claude_line(r#"{"type":"summary","summary":"x"}"#), None);
        assert_eq!(claude_line("not json, \"usage\" \"assistant\""), None);
        let synthetic = CLAUDE.replace("claude-opus-5-5", "<synthetic>");
        assert_eq!(claude_line(&synthetic), None, "Claude Code's own stand-in costs nothing");
    }

    fn codex(lines: &[&str]) -> (Vec<Sample>, CodexFile) {
        let mut file = CodexFile::default();
        let samples = lines.iter().filter_map(|l| codex_line(l, &mut file)).collect();
        (samples, file)
    }

    const META: &str = r#"{"timestamp":"2026-11-01T21:06:10.000Z","type":"session_meta","payload":{"id":"sub","session_id":"root","cwd":"/h/.equipe/worktrees/web/t4-abc","originator":"equipe","base_instructions":{"text":"long"}}}"#;
    const PARENT_META: &str = r#"{"timestamp":"2026-11-01T21:06:10.000Z","type":"session_meta","payload":{"id":"root","cwd":"/elsewhere"}}"#;
    const CONTEXT: &str = r#"{"timestamp":"2026-11-01T21:06:11.000Z","type":"turn_context","payload":{"model":"gpt-5.6-sol","cwd":"/h/.equipe/worktrees/web/t4-abc"}}"#;

    fn count(total_in: u64, cached: u64, out: u64, at: &str) -> String {
        format!(r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total_in},"cached_input_tokens":{cached},"output_tokens":{out},"total_tokens":0}},"last_token_usage":{{}}}}}}}}"#)
    }

    #[test]
    fn codex_totals_count_what_each_adds_once() {
        let (a, b, again) = (count(1000, 600, 50, "2026-11-01T21:06:12Z"), count(2500, 2000, 80, "2026-11-01T21:06:20Z"), count(2500, 2000, 80, "2026-11-01T21:06:30Z"));
        let (samples, file) = codex(&[META, PARENT_META, CONTEXT, &a, &b, &again]);
        assert_eq!(samples.len(), 2, "said again unchanged: nothing new");
        assert_eq!(samples[0].used.tokens, Tokens { input: 400, output: 50, cache_write: 0, cache_read: 600 });
        assert_eq!(samples[1].used.tokens, Tokens { input: 100, output: 30, cache_write: 0, cache_read: 1400 });
        assert_eq!((samples[0].session.as_str(), samples[0].origin.as_str(), samples[0].model.as_str()), ("root", "equipe", "gpt-5.6-sol"), "a subagent's file goes by its root session; the first meta is its own");
        assert_eq!(file.cwd.as_deref(), Some("/h/.equipe/worktrees/web/t4-abc"));
        assert_eq!(file.total, Some([2500, 2000, 0, 80]));
    }

    #[test]
    fn a_rollout_with_request_records_counts_those_and_not_its_totals() {
        let record = r#"{"timestamp":"2026-11-01T21:06:12Z","type":"token_usage_record","payload":{"thread_id":"sub","response_id":"resp_1","usage":{"input_tokens":1000,"cached_input_tokens":600,"cache_write_input_tokens":0,"output_tokens":50}}}"#;
        let (samples, _) = codex(&[META, CONTEXT, record, &count(1000, 600, 50, "2026-11-01T21:06:13Z")]);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].key.as_deref(), Some("codex:resp_1"));
        assert_eq!(samples[0].used.tokens.input, 400);
    }

    #[test]
    fn times_read_as_unix_seconds_whatever_their_zone() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_secs("2026-11-02T15:59:09.215Z"), Some(1_793_635_149));
        assert_eq!(iso_secs("2026-11-03T00:59:09+09:00"), Some(1_793_635_149));
        assert_eq!(iso_secs("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(iso_secs("yesterday"), None);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
    }

    fn total(ledger: &Ledger, engine: &str) -> (Tokens, u64) {
        ledger.hours(engine, 0).unwrap().iter().fold((Tokens::default(), 0), |(mut t, c), (_, u)| {
            t.add(u.tokens);
            (t, c + u.calls)
        })
    }

    #[test]
    fn a_pass_reads_on_from_where_it_left_counting_each_message_once() {
        let dir = std::env::temp_dir().join(format!("equipe-plan-use-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let roots = Roots { claude: Some(dir.join("claude")), codex: Some(dir.join("codex")) };
        let ledger = Ledger::open(&dir.join("plan-use.db")).unwrap();
        let now = 1_793_635_149 + 3600;
        let a = dir.join("claude/-u-code-site/s1.jsonl");
        // One message written once per content block, the second with its final output.
        let grown = CLAUDE.replace("\"output_tokens\":272", "\"output_tokens\":300");
        write(&a, &format!("{CLAUDE}\n{grown}\n"));
        // A file that copies the message (a resumed conversation) and a credential next to it.
        write(&dir.join("claude/-u-code-site/s2.jsonl"), &format!("{CLAUDE}\n"));
        write(&dir.join("codex/auth.json"), r#"{"tokens":"never read"}"#);
        write(&dir.join("codex/2026/11/01/rollout-x.jsonl"), &format!("{META}\n{CONTEXT}\n{}\n", count(1000, 600, 50, "2026-11-02T16:00:00Z")));
        let first = scan(&ledger, &roots, now, None);
        assert!(first.complete);
        assert_eq!(first.files, 3, "the two transcripts and the rollout; nothing else: {first:?}");
        let (claude, calls) = total(&ledger, "claude");
        assert_eq!((claude.output, claude.cache_read, calls), (300, 11347, 1), "once, at its final output");
        // Nothing new: nothing read.
        assert_eq!(scan(&ledger, &roots, now, None).files, 0);
        // A line half written waits; once whole, it is read — and only it.
        let next = CLAUDE.replace("msg_1", "msg_2").replace("req_1", "req_2");
        let (head, tail) = next.split_at(40);
        append(&a, head);
        assert_eq!(scan(&ledger, &roots, now, None).requests, 0);
        append(&a, &format!("{tail}\n"));
        let third = scan(&ledger, &roots, now, None);
        assert_eq!((third.files, third.requests), (1, 1));
        assert_eq!(total(&ledger, "claude").1, 2);
        let (codex, codex_calls) = total(&ledger, "codex");
        assert_eq!((codex.input, codex.cache_read, codex_calls), (400, 600, 1));
        assert!(read_once(&ledger));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_older_than_is_kept_is_let_go() {
        let dir = std::env::temp_dir().join(format!("equipe-plan-use-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let roots = Roots { claude: Some(dir.join("claude")), codex: None };
        let ledger = Ledger::open(&dir.join("plan-use.db")).unwrap();
        write(&dir.join("claude/p/s.jsonl"), &format!("{CLAUDE}\n"));
        // Twenty days on: the file is older than what is kept.
        let later = 1_793_635_149 + 20 * 86_400;
        assert_eq!(scan(&ledger, &roots, later, None).requests, 0);
        assert_eq!(total(&ledger, "claude").1, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
