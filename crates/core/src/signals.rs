//! A project's signals: shell commands that say whether what the squad
//! measures and ships still works — the analytics tag on the site, the
//! workflow that collects traffic, the stars on the repository. Each runs on
//! its own schedule inside a fence, its reading kept (the last
//! [`READINGS`]), and a broken expectation or a big change is told to the PM,
//! which turns a breakage into a ticket or a card at once.
//!
//! The PM proposes a signal (`propose_signal`); only the person's yes on its
//! card turns it on, the exact command shown. Plain functions here, tested
//! without an engine or a shell; the runtime runs and keeps
//! (`runtime/signals.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SIGNALS: &str = "signals";
pub const PROPOSE_SIGNAL: &str = "propose_signal";

/// Readings kept per signal: the newest, oldest dropped first.
pub const READINGS: usize = 90;
/// Signals a squad keeps, across its projects.
pub const MOST: usize = 20;
/// How long a command may run before it is stopped and counted as failing.
pub const TIMEOUT_SECS: u64 = 60;
/// Every six hours, unless the signal says otherwise.
pub const EVERY_DEFAULT: u64 = 6 * 60 * 60;
const EVERY_MIN: u64 = 5 * 60;
const EVERY_MAX: u64 = 7 * 24 * 60 * 60;
/// What a reading keeps of a command's output, at most.
const VALUE_CHARS: usize = 200;

/// What a signal's command is expected to give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expect {
    /// Exits 0.
    Exit0,
    /// Prints a number of at least `value`.
    AtLeast { value: f64 },
    /// Prints a number of at most `value`.
    AtMost { value: f64 },
    /// Prints exactly `value` (trimmed).
    Equals { value: String },
    /// Prints something with `value` in it.
    Contains { value: String },
}

impl Expect {
    /// Shown on the card and to the PM, the same in every language.
    pub fn describe(&self) -> String {
        match self {
            Expect::Exit0 => "exit 0".into(),
            Expect::AtLeast { value } => format!("≥ {}", number_text(*value)),
            Expect::AtMost { value } => format!("≤ {}", number_text(*value)),
            Expect::Equals { value } => format!("= \"{value}\""),
            Expect::Contains { value } => format!("contains \"{value}\""),
        }
    }
}

/// Where a signal reads its value: a shell command, or a product's numbers
/// straight from their API (`crate::signal_sources`), with keys from a file
/// the person saved on an `ask_secret` card (`keys`, `~/…`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// `command`, with `/bin/sh -c` in the project's folder.
    #[default]
    Command,
    /// How many `event` events (all of them when empty) PostHog counted in the last `days` days.
    Posthog {
        project_id: u64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        event: String,
        days: u32,
        keys: String,
    },
    /// App Store Connect first downloads of `app_id` over the last `days` days.
    AppStore { app_id: String, days: u32, keys: String },
}

impl Source {
    pub fn is_command(&self) -> bool {
        matches!(self, Source::Command)
    }
}

/// One signal of one project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// The project's name, as the board names it.
    pub project: String,
    pub name: String,
    /// Run with `/bin/sh -c`, in the project's folder; empty for an API source.
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "Source::is_command")]
    pub source: Source,
    pub expect: Expect,
    /// Told when a number moves this many percent from the reading before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// Seconds between runs.
    pub every: u64,
    #[serde(default)]
    pub added_at: u64,
    /// Paused by the person: kept, not run.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub on: bool,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

impl Signal {
    /// How the journal keys its readings: `project/name`.
    pub fn key(&self) -> String {
        key(&self.project, &self.name)
    }

    /// What it reads, in one line: its command, or the API and what of it.
    pub fn what(&self) -> String {
        match &self.source {
            Source::Command => self.command.clone(),
            Source::Posthog { project_id, event, days, .. } => {
                let event = if event.is_empty() { "all events" } else { event.as_str() };
                format!("PostHog {project_id} · {event} · {days}d")
            }
            Source::AppStore { app_id, days, .. } => format!("App Store {app_id} · downloads · {days}d"),
        }
    }

    /// What it reads, and with which keys: as the card shows it.
    pub fn what_with_keys(&self) -> String {
        match &self.source {
            Source::Command => self.command.clone(),
            Source::Posthog { keys, .. } | Source::AppStore { keys, .. } => format!("{} (keys: {keys})", self.what()),
        }
    }

    /// As screens show it, with its latest readings.
    pub fn view(&self, state: &State) -> crate::protocol::signals::SignalView {
        use crate::protocol::signals::{SignalReading, SignalSource, SignalView, VIEW_READINGS};
        let skip = state.readings.len().saturating_sub(VIEW_READINGS);
        SignalView {
            project: self.project.clone(),
            name: self.name.clone(),
            source: match self.source {
                Source::Command => SignalSource::Command,
                Source::Posthog { .. } => SignalSource::Posthog,
                Source::AppStore { .. } => SignalSource::AppStore,
            },
            what: self.what(),
            expect: self.expectation(),
            every: self.every,
            on: self.on,
            readings: state.readings[skip..].iter().map(|r| SignalReading { at: r.at, ok: r.ok, value: r.value.clone(), note: r.note.clone() }).collect(),
        }
    }

    /// The expectation, with the change it is told about.
    pub fn expectation(&self) -> String {
        match self.change_pct {
            Some(pct) => format!("{}, Δ ≥ {}%", self.expect.describe(), number_text(pct)),
            None => self.expect.describe(),
        }
    }
}

pub fn key(project: &str, name: &str) -> String {
    format!("{project}/{name}")
}

/// One run of a signal's command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub at: u64,
    pub ok: bool,
    /// What it printed, trimmed, on one line, clipped, anything like a key blanked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Why it fails, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A signal's readings, and whether its failing was told already.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub readings: Vec<Reading>,
    /// Told to the PM as failing and not since seen passing.
    #[serde(default)]
    pub alerted: bool,
}

/// What a command did: its exit code (none: stopped, or never started) and output.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// It ran past [`TIMEOUT_SECS`], or couldn't run at all: why.
    pub failed: Option<String>,
}

/// The reading `ran` makes against `expect`.
pub fn judge(expect: &Expect, ran: &Ran, at: u64) -> Reading {
    let value = Some(one_line(&ran.stdout)).filter(|v| !v.is_empty());
    let fail = |note: String| Reading { at, ok: false, value: value.clone(), note: Some(note) };
    if let Some(why) = &ran.failed {
        return fail(why.clone());
    }
    let exited = || match ran.exit {
        Some(code) => {
            let said = one_line(ran.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default());
            if said.is_empty() { format!("exit {code}") } else { format!("exit {code}: {}", clip(&said, 160)) }
        }
        None => "stopped".into(),
    };
    let ok = match expect {
        Expect::Exit0 => {
            if ran.exit == Some(0) {
                true
            } else {
                return fail(exited());
            }
        }
        Expect::AtLeast { value: want } | Expect::AtMost { value: want } => {
            // `grep -c` says 0 with exit 1: the number is what counts, when there is one.
            let Some(got) = value.as_deref().and_then(number) else {
                return fail(if ran.exit == Some(0) { "no number in the output".into() } else { exited() });
            };
            let ok = if matches!(expect, Expect::AtLeast { .. }) { got >= *want } else { got <= *want };
            if !ok {
                let op = if matches!(expect, Expect::AtLeast { .. }) { "<" } else { ">" };
                return fail(format!("{} {op} {}", number_text(got), number_text(*want)));
            }
            true
        }
        Expect::Equals { value: want } | Expect::Contains { value: want } => {
            let got = value.clone().unwrap_or_default();
            let ok = if matches!(expect, Expect::Equals { .. }) { got == want.trim() } else { got.contains(want.as_str()) };
            if !ok {
                return fail(match (got.is_empty(), ran.exit) {
                    (true, Some(code)) if code != 0 => exited(),
                    (true, _) => "no output".into(),
                    _ => format!("\"{}\", not {}", clip(&got, 60), expect.describe()),
                });
            }
            true
        }
    };
    Reading { at, ok, value, note: None }
}

/// The number a value starts with: `1,234`, `25`, `-3.5 stars`.
pub fn number(value: &str) -> Option<f64> {
    let first = value.split_whitespace().next()?.trim_matches('"').replace(',', "");
    first.parse().ok().filter(|n: &f64| n.is_finite())
}

fn number_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n:.2}") }
}

/// How far a number moved from the reading before, in percent; none when
/// either isn't a number or both are 0. From 0: infinitely.
pub fn change(before: &Reading, now: &Reading) -> Option<f64> {
    let (b, n) = (before.value.as_deref().and_then(number)?, now.value.as_deref().and_then(number)?);
    match (b == 0.0, n == 0.0) {
        (true, true) => None,
        (true, false) => Some(f64::INFINITY.copysign(n)),
        _ => Some((n - b) / b.abs() * 100.0),
    }
}

/// What a new reading means for the PM.
#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    Quiet,
    /// Failing, and not told before: the PM is woken.
    Failing,
    /// Passing again after it was told failing.
    Recovered,
    /// Passing, and its number moved by `pct` from `from`.
    Changed { pct: f64, from: String },
}

/// Keep `reading` (the last [`READINGS`]) and say what it means. A failure
/// with no PM to tell waits to be told at the next reading.
pub fn step(signal: &Signal, state: &mut State, reading: Reading, pm_here: bool) -> Alert {
    let before = state.readings.last().cloned();
    let ok = reading.ok;
    state.readings.push(reading.clone());
    if state.readings.len() > READINGS {
        let extra = state.readings.len() - READINGS;
        state.readings.drain(..extra);
    }
    if !ok {
        if state.alerted || !pm_here {
            return Alert::Quiet;
        }
        state.alerted = true;
        return Alert::Failing;
    }
    if state.alerted {
        state.alerted = false;
        return Alert::Recovered;
    }
    let (Some(threshold), Some(before)) = (signal.change_pct, before.filter(|b| b.ok)) else { return Alert::Quiet };
    match change(&before, &reading) {
        Some(pct) if pct.abs() >= threshold => Alert::Changed { pct, from: before.value.unwrap_or_default() },
        _ => Alert::Quiet,
    }
}

/// Whether a signal last read at `last` runs again at `now`. Never read: now.
pub fn due(signal: &Signal, last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|at| now >= at.saturating_add(signal.every))
}

/// `30m`, `6h`, `1d` in seconds, between five minutes and a week.
pub fn parse_every(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let bad = || format!("`every` is a number and a unit — 30m, 6h, 1d — not \"{text}\".");
    let split = text.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
    let n: u64 = text[..split].parse().map_err(|_| bad())?;
    let unit = match &text[split..] {
        "m" | "min" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return Err(bad()),
    };
    let secs = n.saturating_mul(unit);
    if !(EVERY_MIN..=EVERY_MAX).contains(&secs) {
        return Err("A signal runs between every 5 minutes (5m) and once a week (7d).".into());
    }
    Ok(secs)
}

/// Seconds as `every` writes them: `6h`, `1d`, `30m`.
pub fn every_text(secs: u64) -> String {
    match secs {
        s if s % (24 * 60 * 60) == 0 => format!("{}d", s / (24 * 60 * 60)),
        s if s % (60 * 60) == 0 => format!("{}h", s / (60 * 60)),
        s => format!("{}m", s / 60),
    }
}

/// What a `propose_signal` call asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A signal turned on, or one of the same name changed.
    On { signal: Signal, replaces: bool },
    Off { project: String, name: String },
}

/// A `propose_signal` call, checked against the signals kept and the
/// squad's `projects` (by name): what goes on the card, and why.
pub fn check(args: &Value, kept: &[Signal], projects: &[String], home: &Path, now: u64) -> Result<(Change, String), String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let name = text("name");
    if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Give the signal a short `name` of letters, digits and dashes: site-analytics.".into());
    }
    let project = match (text("project"), projects) {
        (p, _) if !p.is_empty() => projects.iter().find(|n| n.eq_ignore_ascii_case(&p)).cloned().ok_or_else(|| {
            format!("\"{p}\" isn't one of the squad's projects: {}.", projects.join(", "))
        })?,
        (_, [only]) => only.clone(),
        _ => return Err(format!("Say which `project` it watches: {}.", projects.join(", "))),
    };
    let reason = text("reason");
    let existing = kept.iter().find(|s| s.project == project && s.name == name);
    if args.get("remove").and_then(Value::as_bool) == Some(true) {
        return match existing {
            Some(_) => Ok((Change::Off { project, name }, reason)),
            None => Err(format!("{project} has no signal \"{name}\" to turn off.")),
        };
    }
    let source = source(args, home)?;
    let command = if source.is_command() { text("command") } else { String::new() };
    if source.is_command() {
        if command.is_empty() {
            return Err("Give the `command`: a shell command that prints the value, or exits 0 when all is well.".into());
        }
        if command.len() > 2000 {
            return Err("Keep the command under 2000 characters; put a longer one in a script in the project.".into());
        }
        if crate::permissions::blocked("Bash", &command) {
            return Err("equipe never runs that command, in any mode.".into());
        }
    }
    let wanted = || args.get("value").and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(number)));
    let said = || args.get("value").map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).filter(|v| !v.trim().is_empty());
    let expect = match text("expect").as_str() {
        // An API's count: passing whenever it can be read.
        "" if !source.is_command() => Expect::AtLeast { value: 0.0 },
        "" | "exit0" | "exit_0" => Expect::Exit0,
        "at_least" => Expect::AtLeast { value: wanted().ok_or("`at_least` needs a number `value`.")? },
        "at_most" => Expect::AtMost { value: wanted().ok_or("`at_most` needs a number `value`.")? },
        "equals" => Expect::Equals { value: said().ok_or("`equals` needs the `value` it prints.")? },
        "contains" => Expect::Contains { value: said().ok_or("`contains` needs the `value` to look for.")? },
        other => return Err(format!("`expect` is exit0, at_least, at_most, equals or contains — not \"{other}\".")),
    };
    let change_pct = match args.get("change_pct") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().filter(|p| *p > 0.0).ok_or("`change_pct` is a percent above 0: 20 tells of a 20% move.")?),
    };
    let every = match text("every").as_str() {
        "" => EVERY_DEFAULT,
        every => parse_every(every)?,
    };
    if existing.is_none() && kept.len() >= MOST {
        return Err(format!("A squad keeps up to {MOST} signals; propose turning one off first."));
    }
    let signal = Signal { project, name, command, source, expect, change_pct, every, added_at: now, on: true };
    if let Some(e) = existing.filter(|e| Signal { added_at: now, on: true, ..(*e).clone() } == signal) {
        return Err(if e.on {
            format!("\"{}\" is already on, just so.", signal.name)
        } else {
            format!("\"{}\" is kept just so, paused by the person: they start it again in Settings › Signals.", signal.name)
        });
    }
    Ok((Change::On { signal, replaces: existing.is_some() }, reason))
}

/// Where a proposed signal reads its value, checked: an API source's keys'
/// file must be one `ask_secret` may save to, under the person's `home`.
fn source(args: &Value, home: &Path) -> Result<Source, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let kind = text("source");
    if matches!(kind.as_str(), "" | "command") {
        return Ok(Source::Command);
    }
    let keys = text("keys");
    if keys.is_empty() {
        return Err("Say in `keys` which file holds the API keys, as saved with `ask_secret`: ~/.config/<project>/<name>.env. Never put a key in the signal itself.".into());
    }
    crate::secrets::place(&keys, home).map_err(|why| format!("`keys`: {why}"))?;
    let days = match args.get("days") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().filter(|d| (1..=90).contains(d)).ok_or("`days` is a whole number of days, 1 to 90.")? as u32,
    };
    match kind.as_str() {
        "posthog" => {
            let project_id = args.get("posthog_project").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
            let project_id = project_id.ok_or("Give `posthog_project`: the PostHog project's number (in its URL, /project/<number>).")?;
            let event = text("event");
            let fine = |c: char| c.is_ascii_alphanumeric() || "$_-.: /".contains(c);
            if event.len() > 100 || !event.chars().all(fine) {
                return Err("`event` is a PostHog event's name — $pageview, signed_up — or left out for all events.".into());
            }
            Ok(Source::Posthog { project_id, event, days, keys })
        }
        "app_store" => {
            let app_id = args.get("app_id").map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).unwrap_or_default();
            let app_id = app_id.trim().to_owned();
            if app_id.is_empty() || app_id.len() > 20 || !app_id.chars().all(|c| c.is_ascii_digit()) {
                return Err("Give `app_id`: the app's Apple ID, the number in App Store Connect › App Information.".into());
            }
            if days > 31 {
                return Err("App Store downloads are read a day at a time: `days` up to 31.".into());
            }
            Ok(Source::AppStore { app_id, days, keys })
        }
        other => Err(format!("`source` is command, posthog or app_store — not \"{other}\".")),
    }
}

/// The person pauses `project/name` (`on` false) or starts it again: whether
/// there was such a signal to change.
pub fn set_on(kept: &mut [Signal], project: &str, name: &str, on: bool) -> bool {
    match kept.iter_mut().find(|s| s.project == project && s.name == name) {
        Some(signal) => {
            signal.on = on;
            true
        }
        None => false,
    }
}

/// The person's yes applied to the signals kept.
pub fn apply(kept: &mut Vec<Signal>, change: &Change) {
    match change {
        Change::On { signal, .. } => match kept.iter_mut().find(|s| s.project == signal.project && s.name == signal.name) {
            Some(old) => *old = signal.clone(),
            None => kept.push(signal.clone()),
        },
        Change::Off { project, name } => kept.retain(|s| !(&s.project == project && &s.name == name)),
    }
}

/// The card's payload, kept with it until the person answers.
pub fn card_input(change: &Change) -> Value {
    match change {
        Change::On { signal, replaces } => json!({ "signal_on": signal, "replaces": replaces }),
        Change::Off { project, name } => json!({ "signal_off": { "project": project, "name": name } }),
    }
}

/// The change a card was about, if it was a signal's.
pub fn card_change(input: &Value) -> Option<Change> {
    if let Some(signal) = input.get("signal_on").and_then(|s| serde_json::from_value(s.clone()).ok()) {
        let replaces = input.get("replaces").and_then(Value::as_bool).unwrap_or(false);
        return Some(Change::On { signal, replaces });
    }
    let off = input.get("signal_off")?;
    let text = |k: &str| off.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(Change::Off { project: text("project")?, name: text("name")? })
}

/// What the output keeps: trimmed, on one line, clipped, keys blanked.
pub fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&redact(&joined), VALUE_CHARS)
}

fn clip(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(most).collect();
    cut.push('…');
    cut
}

/// Anything that looks like a key or a token — a long unbroken run of
/// letters and digits, or one with a known key's prefix — blanked.
pub fn redact(text: &str) -> String {
    const PREFIXES: [&str; 10] = ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "sk-", "xox", "phc_", "phx_", "AKIA"];
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let secret = run.len() >= 32 || (run.len() >= 12 && PREFIXES.iter().any(|p| run.starts_with(p)));
        out.push_str(if secret { "[redacted]" } else { run.as_str() });
        run.clear();
    };
    for c in text.chars() {
        if word(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// The fence a signal's command runs in (macOS `sandbox-exec`): it writes
/// nowhere but `scratch` and reads none of `private` — the person's sign-ins,
/// equipe's logins, other squads' folders; the network is open, as the
/// person saw the command on the card.
pub fn fence_profile(scratch: &Path, private: &[PathBuf]) -> String {
    let quote = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    profile.push_str(&format!(
        "(allow file-write* (subpath {}) (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\") (regex #\"^/dev/fd/\"))\n",
        quote(scratch)
    ));
    if !private.is_empty() {
        let paths: Vec<String> = private.iter().map(|p| format!("(subpath {})", quote(p))).collect();
        profile.push_str(&format!("(deny file-read* {})\n", paths.join(" ")));
    }
    profile
}

/// A failure as the PM is woken with it.
pub fn failing_message(signal: &Signal, reading: &Reading, passed_before: Option<u64>) -> String {
    let since = match passed_before {
        Some(at) => format!("It last passed at {}.", time_text(at)),
        None => "It hasn't passed since it was turned on.".into(),
    };
    format!(
        "[Signal · {name} · failing]\n\nThe signal \"{name}\" of {project} fails: {note}.\nReads: `{command}`\nExpected: {expect}; got: {got}. {since}\n\n\
This means something the squad measures or ships is broken, and nobody else has noticed. Act on it now: if the squad can \
fix it, put up a ticket to fix it (`create_ticket` for the member who owns that part, or a proposal when it needs the \
person's go). If it needs the person — a token, a secret, a setting, an account — put it in front of them as a card at \
once (`block_on_person` on that ticket, `ask_secret`, or `ask_person`), one item each. Don't only report it in words. If \
the signal itself is wrong, propose changing it with `propose_signal`. `signals` shows its readings.",
        name = signal.name,
        project = signal.project,
        note = reading.note.as_deref().unwrap_or("failed"),
        command = signal.what(),
        expect = signal.expectation(),
        got = reading.value.as_deref().map_or_else(|| "nothing".into(), |v| format!("\"{v}\"")),
    )
}

/// Passing again: a line for the PM's next message.
pub fn recovered_line(signal: &Signal, reading: &Reading) -> String {
    format!(
        "The signal \"{}\" of {} passes again ({}). If a ticket or a card was up for it, close it or tell the person.",
        signal.name,
        signal.project,
        reading.value.as_deref().unwrap_or("exit 0")
    )
}

/// A big move: a line for the PM's next message.
pub fn changed_line(signal: &Signal, from: &str, reading: &Reading, pct: f64) -> String {
    format!(
        "The signal \"{}\" of {} moved {}: {from} → {}. Look at why if it matters to the squad's goal, and bring it up at \
the next check-in.",
        signal.name,
        signal.project,
        pct_text(pct),
        reading.value.as_deref().unwrap_or_default()
    )
}

pub fn pct_text(pct: f64) -> String {
    if pct.is_infinite() { "from 0".into() } else { format!("{:+.0}%", pct) }
}

/// `MM-DD HH:MM` in this machine's local time.
pub fn time_text(at: u64) -> String {
    use crate::routines::Clock as _;
    let p = crate::routines::Local.parts(at);
    format!("{:02}-{:02} {:02}:{:02}", p.month, p.day, p.hour, p.minute)
}

/// What the `signals` tool reads out: each signal, its state, its last
/// reading and the trend of its last ten; with `only`, all its readings.
pub fn report(signals: &[Signal], state: impl Fn(&Signal) -> State, only: Option<&str>) -> String {
    if signals.is_empty() {
        return "No signals yet. When something the squad measures or ships could break unnoticed — an analytics tag, a \
workflow, a store listing — propose a signal with `propose_signal`; the person turns it on with one click."
            .into();
    }
    let mut out = String::new();
    for signal in signals.iter().filter(|s| only.is_none_or(|n| s.name == n)) {
        let st = state(signal);
        let last = st.readings.last();
        let status = match last {
            _ if !signal.on => "paused by the person".to_owned(),
            None => "not read yet".to_owned(),
            Some(r) if r.ok => "passing".to_owned(),
            Some(_) => {
                let since = st.readings.iter().rev().take_while(|r| !r.ok).last().map_or(0, |r| r.at);
                format!("FAILING since {}", time_text(since))
            }
        };
        out.push_str(&format!(
            "{} · {} — {status}\n  reads: {}\n  expected: {}; every {}\n",
            signal.project,
            signal.name,
            signal.what(),
            signal.expectation(),
            every_text(signal.every)
        ));
        if let Some(r) = last {
            let note = r.note.as_deref().map(|n| format!(" — {n}")).unwrap_or_default();
            out.push_str(&format!("  last: {} at {}{note}\n", r.value.as_deref().unwrap_or("(no output)"), time_text(r.at)));
        }
        let shown: Vec<&Reading> = if only.is_some() { st.readings.iter().collect() } else { st.readings.iter().rev().take(10).rev().collect() };
        if only.is_some() {
            for r in shown {
                let mark = if r.ok { "ok" } else { "FAIL" };
                out.push_str(&format!("  {} {mark} {}\n", time_text(r.at), r.value.as_deref().unwrap_or("-")));
            }
        } else if shown.len() > 1 {
            let values: Vec<String> = shown.iter().map(|r| format!("{}{}", r.value.as_deref().unwrap_or("-"), if r.ok { "" } else { "✗" })).collect();
            let moved = match (st.readings.first(), last) {
                (Some(first), Some(last)) if st.readings.len() > 1 => change(first, last).map(|p| format!(" ({} over {} readings)", pct_text(p), st.readings.len())),
                _ => None,
            };
            out.push_str(&format!("  trend: {}{}\n", values.join(", "), moved.unwrap_or_default()));
        }
    }
    if out.is_empty() {
        return format!("No signal is called \"{}\".", only.unwrap_or_default());
    }
    out
}

/// The PM's tools.
pub fn tools() -> [Value; 2] {
    [
        json!({
            "name": SIGNALS,
            "description": "Read the squad's signals: each project's checks of what it measures and ships (an analytics \
tag on the site, a workflow that collects data, a count of stars), whether each passes, its last value and its trend. Use \
it at every check-in and whenever you report on the product's numbers. `name` reads all of one signal's readings (up to \
90); `check` runs them now, and what fails or moves reaches you as a briefing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "One signal, with all its readings." },
                    "check": { "type": "boolean", "description": "Run them (or the one named) now." },
                },
            },
        }),
        json!({
            "name": PROPOSE_SIGNAL,
            "description": "Propose a signal for a project — a shell command equipe runs on a schedule to see whether \
something the squad relies on still works, or what a number does — for the person to turn on with one click on a card that \
shows the exact command. Only the person turns one on. It runs in the project's folder inside a fence (it writes nowhere, \
reads no sign-ins); `gh`, `curl` and the project's own tools work as they are signed in. Never make a command print a \
secret: a reading keeps what it prints. When a signal fails you are woken to put up a ticket or a card. `remove` proposes \
turning one off. Examples: `curl -s https://example.com/ | grep -c analytics-config`, at_least 1; `gh run list -R org/repo \
-L1 --json conclusion --jq '.[0].conclusion'`, equals success; `gh api repos/org/repo --jq .stargazers_count`, at_least 0 \
with change_pct 20. For a product's numbers use an API source instead of a command — `source` posthog (events counted) or \
app_store (downloads) — with the keys in a file the person saved on an `ask_secret` card (`keys`); equipe reads the API \
itself and the key never appears in the signal, the card or a reading. If the file isn't saved yet, ask for it with \
`ask_secret` first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "The project it watches; the only one when left out." },
                    "name": { "type": "string", "description": "Short, letters, digits and dashes: site-analytics." },
                    "source": { "type": "string", "enum": ["command", "posthog", "app_store"], "description": "command when left out. posthog: a count of events; app_store: first downloads. Both read with keys from `keys`." },
                    "command": { "type": "string", "description": "For source command." },
                    "posthog_project": { "type": "integer", "description": "For posthog: the project's number." },
                    "event": { "type": "string", "description": "For posthog: the event to count ($pageview); all events when left out." },
                    "app_id": { "type": "string", "description": "For app_store: the app's Apple ID (a number)." },
                    "days": { "type": "integer", "description": "For posthog and app_store: the days counted, back from now (1 when left out; App Store up to 31, ending the day before yesterday)." },
                    "keys": { "type": "string", "description": "For posthog and app_store: the file the person saved the keys to with `ask_secret`, ~/.config/<project>/<name>.env. PostHog: POSTHOG_PERSONAL_API_KEY (a personal key with query read), POSTHOG_HOST if not US cloud. App Store: ASC_KEY_ID, ASC_ISSUER_ID, ASC_VENDOR_NUMBER, and ASC_KEY_FILE (the .p8's path) or ASC_PRIVATE_KEY — a key with the Sales role." },
                    "expect": { "type": "string", "enum": ["exit0", "at_least", "at_most", "equals", "contains"], "description": "exit0 when left out." },
                    "value": { "description": "The number, or the text, `expect` compares with." },
                    "change_pct": { "type": "number", "description": "Also tell when its number moves this many percent from the reading before." },
                    "every": { "type": "string", "description": "30m, 6h, 1d — 6h when left out." },
                    "remove": { "type": "boolean", "description": "Propose turning the signal named off." },
                    "reason": { "type": "string", "description": "Why, in one line in the person's language." },
                },
                "required": ["name"],
            },
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(expect: Expect, change_pct: Option<f64>) -> Signal {
        Signal { project: "site".into(), name: "s".into(), command: "true".into(), source: Source::Command, expect, change_pct, every: 3600, added_at: 0, on: true }
    }

    fn ran(exit: i32, stdout: &str) -> Ran {
        Ran { exit: Some(exit), stdout: stdout.into(), ..Ran::default() }
    }

    fn ok(at: u64, value: &str) -> Reading {
        Reading { at, ok: true, value: Some(value.into()), note: None }
    }

    #[test]
    fn a_reading_passes_or_fails_by_what_was_expected() {
        let at_least = Expect::AtLeast { value: 1.0 };
        assert!(judge(&at_least, &ran(0, "3\n"), 1).ok);
        // `grep -c` finding nothing says 0 and exits 1: failing by its number.
        let none = judge(&at_least, &ran(1, "0\n"), 1);
        assert_eq!((none.ok, none.value.as_deref(), none.note.as_deref()), (false, Some("0"), Some("0 < 1")));
        let broken = judge(&at_least, &Ran { exit: Some(127), stderr: "sh: curl: not found\n".into(), ..Ran::default() }, 1);
        assert_eq!(broken.note.as_deref(), Some("exit 127: sh: curl: not found"));
        assert!(!judge(&Expect::AtMost { value: 5.0 }, &ran(0, "1,234"), 1).ok);
        assert!(judge(&Expect::Exit0, &ran(0, ""), 1).ok);
        assert_eq!(judge(&Expect::Exit0, &Ran { exit: Some(1), stderr: "GH_TRAFFIC_TOKEN is not set".into(), ..Ran::default() }, 1).note.as_deref(), Some("exit 1: GH_TRAFFIC_TOKEN is not set"));
        let success = Expect::Equals { value: "success".into() };
        assert!(judge(&success, &ran(0, "success\n"), 1).ok);
        assert_eq!(judge(&success, &ran(0, "failure"), 1).note.as_deref(), Some("\"failure\", not = \"success\""));
        assert!(judge(&Expect::Contains { value: "success".into() }, &ran(0, "[{\"conclusion\":\"success\"}]"), 1).ok);
        let late = judge(&success, &Ran { failed: Some("timed out after 60s".into()), ..Ran::default() }, 1);
        assert_eq!((late.ok, late.note.as_deref()), (false, Some("timed out after 60s")));
    }

    #[test]
    fn readings_keep_the_last_ninety_and_a_failure_is_told_once_until_it_passes() {
        let s = signal(Expect::AtLeast { value: 1.0 }, None);
        let mut state = State::default();
        for at in 0..100 {
            assert_eq!(step(&s, &mut state, ok(at, "2"), true), Alert::Quiet);
        }
        assert_eq!(state.readings.len(), READINGS);
        assert_eq!(state.readings[0].at, 10, "the oldest dropped first");
        let fail = |at| Reading { at, ok: false, value: Some("0".into()), note: Some("0 < 1".into()) };
        // No PM to tell yet: told at the next reading, when there is one.
        assert_eq!(step(&s, &mut state, fail(100), false), Alert::Quiet);
        assert_eq!(step(&s, &mut state, fail(101), true), Alert::Failing);
        assert_eq!(step(&s, &mut state, fail(102), true), Alert::Quiet, "told once");
        assert_eq!(step(&s, &mut state, ok(103, "1"), true), Alert::Recovered);
        assert_eq!(step(&s, &mut state, fail(104), true), Alert::Failing, "broken again: told again");
    }

    #[test]
    fn a_big_move_from_the_reading_before_is_told_and_a_small_one_isnt() {
        let s = signal(Expect::AtLeast { value: 0.0 }, Some(20.0));
        let mut state = State::default();
        assert_eq!(step(&s, &mut state, ok(1, "100"), true), Alert::Quiet, "nothing before it");
        assert_eq!(step(&s, &mut state, ok(2, "110"), true), Alert::Quiet, "+10%");
        assert_eq!(step(&s, &mut state, ok(3, "132"), true), Alert::Changed { pct: 20.0, from: "110".into() });
        assert_eq!(step(&s, &mut state, ok(4, "66"), true), Alert::Changed { pct: -50.0, from: "132".into() });
        assert_eq!(change(&ok(0, "0"), &ok(1, "5")), Some(f64::INFINITY));
        assert_eq!(change(&ok(0, "0"), &ok(1, "0")), None);
        assert_eq!(change(&ok(0, "n/a"), &ok(1, "5")), None);
        // Without a threshold nothing is told of moves.
        let plain = signal(Expect::AtLeast { value: 0.0 }, None);
        let mut state = State::default();
        step(&plain, &mut state, ok(1, "100"), true);
        assert_eq!(step(&plain, &mut state, ok(2, "1"), true), Alert::Quiet);
        assert_eq!(pct_text(-50.0), "-50%");
        assert_eq!(pct_text(20.0), "+20%");
    }

    #[test]
    fn a_signal_runs_when_its_time_has_come() {
        let s = signal(Expect::Exit0, None);
        assert!(due(&s, None, 0));
        assert!(!due(&s, Some(1000), 1000 + 3599));
        assert!(due(&s, Some(1000), 1000 + 3600));
        assert_eq!(parse_every("6h"), Ok(6 * 3600));
        assert_eq!(parse_every("30m"), Ok(1800));
        assert_eq!(parse_every("1d"), Ok(86400));
        assert!(parse_every("1m").is_err() && parse_every("8d").is_err() && parse_every("often").is_err());
        assert_eq!((every_text(21600), every_text(86400), every_text(1800)), ("6h".into(), "1d".into(), "30m".into()));
    }

    #[test]
    fn a_proposed_signal_is_checked_before_it_reaches_a_card() {
        let projects = vec!["equipe-site".to_owned(), "equipe".to_owned()];
        let args = json!({ "project": "equipe-site", "name": "site-analytics", "command": "curl -s https://equipe.im/ | grep -c analytics-config", "expect": "at_least", "value": 1, "reason": "PostHog" });
        let (change, why) = check(&args, &[], &projects, Path::new("/h"), 7).unwrap();
        let Change::On { signal, replaces: false } = &change else { panic!("{change:?}") };
        assert_eq!((signal.expect.clone(), signal.every, why.as_str()), (Expect::AtLeast { value: 1.0 }, EVERY_DEFAULT, "PostHog"));
        assert_eq!(card_change(&card_input(&change)), Some(change.clone()));
        let mut kept = Vec::new();
        apply(&mut kept, &change);
        assert_eq!(kept.len(), 1);
        assert!(check(&args, &kept, &projects, Path::new("/h"), 8).unwrap_err().contains("already on"));
        let changed = json!({ "project": "equipe-site", "name": "site-analytics", "command": "true", "every": "1h" });
        assert!(matches!(check(&changed, &kept, &projects, Path::new("/h"), 8), Ok((Change::On { replaces: true, .. }, _))));
        let off = check(&json!({ "project": "equipe-site", "name": "site-analytics", "remove": true }), &kept, &projects, Path::new("/h"), 9).unwrap().0;
        assert_eq!(card_change(&card_input(&off)), Some(off.clone()));
        apply(&mut kept, &off);
        assert!(kept.is_empty());

        let bad = |args: Value| check(&args, &[], &projects, Path::new("/h"), 0).unwrap_err();
        assert!(bad(json!({ "project": "equipe-site", "name": "a b", "command": "true" })).contains("name"));
        assert!(bad(json!({ "name": "x", "command": "true" })).contains("which `project`"));
        assert!(bad(json!({ "project": "nope", "name": "x", "command": "true" })).contains("isn't one of"));
        assert!(bad(json!({ "project": "equipe", "name": "x", "command": "sudo rm -rf /" })).contains("never"));
        assert!(bad(json!({ "project": "equipe", "name": "x", "command": "true", "expect": "at_least" })).contains("number"));
        assert!(bad(json!({ "project": "equipe", "name": "x", "command": "true", "expect": "big" })).contains("expect"));
        assert!(bad(json!({ "project": "equipe", "name": "x", "remove": true })).contains("no signal"));
        // One project: it needn't be named.
        assert!(check(&json!({ "name": "x", "command": "true" }), &[], &["only".into()], Path::new("/h"), 0).is_ok());
    }

    #[test]
    fn an_api_signal_names_its_keys_file_never_a_key() {
        let home = Path::new("/h");
        let projects = vec!["equipe-site".to_owned()];
        let args = json!({ "name": "pageviews", "source": "posthog", "posthog_project": 700951, "event": "$pageview", "days": 7, "keys": "~/.config/equipe-site/analytics.env", "change_pct": 30 });
        let (change, _) = check(&args, &[], &projects, home, 1).unwrap();
        let Change::On { signal, .. } = &change else { panic!("{change:?}") };
        assert_eq!(signal.source, Source::Posthog { project_id: 700951, event: "$pageview".into(), days: 7, keys: "~/.config/equipe-site/analytics.env".into() });
        assert_eq!((signal.command.as_str(), &signal.expect), ("", &Expect::AtLeast { value: 0.0 }), "a count passes whenever it is read");
        assert_eq!(signal.what(), "PostHog 700951 · $pageview · 7d");
        assert_eq!(signal.what_with_keys(), "PostHog 700951 · $pageview · 7d (keys: ~/.config/equipe-site/analytics.env)");
        assert_eq!(card_change(&card_input(&change)), Some(change.clone()), "the card carries it as it was checked");
        let store = json!({ "name": "downloads", "source": "app_store", "app_id": "6740000000", "keys": "~/.config/equipe-app/asc.env" });
        let Change::On { signal, .. } = check(&store, &[], &projects, home, 1).unwrap().0 else { panic!() };
        assert_eq!(signal.what(), "App Store 6740000000 · downloads · 1d");
        let bad = |args: Value| check(&args, &[], &projects, home, 0).unwrap_err();
        assert!(bad(json!({ "name": "x", "source": "posthog", "posthog_project": 1 })).contains("`keys`"));
        assert!(bad(json!({ "name": "x", "source": "posthog", "posthog_project": 1, "keys": "~/.ssh/id" })).contains("`keys`"));
        assert!(bad(json!({ "name": "x", "source": "posthog", "keys": "~/.config/a.env" })).contains("posthog_project"));
        assert!(bad(json!({ "name": "x", "source": "posthog", "posthog_project": 1, "event": "x' OR 1=1", "keys": "~/.config/a.env" })).contains("event"));
        assert!(bad(json!({ "name": "x", "source": "app_store", "app_id": "abc", "keys": "~/.config/a.env" })).contains("app_id"));
        assert!(bad(json!({ "name": "x", "source": "app_store", "app_id": "1", "days": 60, "keys": "~/.config/a.env" })).contains("31"));
        assert!(bad(json!({ "name": "x", "source": "sql", "keys": "~/.config/a.env" })).contains("source"));
    }

    #[test]
    fn a_paused_signal_is_kept_and_shown_paused() {
        let mut kept = vec![signal(Expect::AtLeast { value: 1.0 }, None)];
        assert!(set_on(&mut kept, "site", "s", false));
        assert!(!set_on(&mut kept, "site", "nope", false));
        assert!(!kept[0].on);
        let json = serde_json::to_value(&kept[0]).unwrap();
        assert_eq!(serde_json::from_value::<Signal>(json).unwrap(), kept[0]);
        let state = State { readings: (0..40).map(|at| ok(at, "3")).collect(), alerted: false };
        let view = kept[0].view(&state);
        assert_eq!((view.on, view.readings.len(), view.readings[0].at), (false, crate::protocol::signals::VIEW_READINGS, 10));
        assert!(report(&kept, |_| state.clone(), None).contains("paused by the person"));
        // Kept from before there were sources: a command, on.
        let old: Signal = serde_json::from_str(r#"{"project":"p","name":"n","command":"true","expect":{"kind":"exit0"},"every":60}"#).unwrap();
        assert!(old.on && old.source.is_command());
    }

    #[test]
    fn what_a_reading_keeps_is_one_clipped_line_with_keys_blanked() {
        assert_eq!(one_line("  25\n\n"), "25");
        assert_eq!(one_line("token ghp_abcdefghijklmnop1234 ok"), "token [redacted] ok");
        assert_eq!(one_line("k=0123456789abcdef0123456789abcdef01"), "k=[redacted]");
        assert_eq!(redact("success"), "success");
        assert_eq!(one_line(&"x ".repeat(300)).chars().count(), VALUE_CHARS + 1);
        assert_eq!(number("1,234 stars"), Some(1234.0));
        assert_eq!(number("\"25\""), Some(25.0));
        assert_eq!(number("success"), None);
    }

    #[test]
    fn the_fence_writes_only_to_its_scratch_and_reads_no_private_folder() {
        let profile = fence_profile(Path::new("/tmp/s\"x"), &[PathBuf::from("/h/.equipe/credentials")]);
        assert!(profile.contains("(deny file-write*)"), "{profile}");
        assert!(profile.contains("(allow file-write* (subpath \"/tmp/s\\\"x\")"), "{profile}");
        assert!(profile.contains("(deny file-read* (subpath \"/h/.equipe/credentials\"))"), "{profile}");
    }

    #[test]
    fn the_pm_is_told_what_broke_and_to_put_up_a_ticket_or_a_card() {
        let s = signal(Expect::AtLeast { value: 1.0 }, None);
        let r = Reading { at: 5, ok: false, value: Some("0".into()), note: Some("0 < 1".into()) };
        let text = failing_message(&s, &r, None);
        assert!(text.starts_with("[Signal · s · failing]"), "{text}");
        assert!(text.contains("`create_ticket`") && text.contains("`block_on_person`") && text.contains("0 < 1"), "{text}");
        let report = report(std::slice::from_ref(&s), |_| State { readings: vec![ok(1, "1"), r.clone()], alerted: true }, None);
        assert!(report.contains("site · s — FAILING since") && report.contains("trend: 1, 0✗"), "{report}");
        assert!(super::report(&[], |_| State::default(), None).contains("propose_signal"));
    }
}
