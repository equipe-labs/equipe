//! How the conversation's activity reads: tool calls gathered into groups,
//! each group in one line (what it was for, or a count of what it did), each
//! call as a verb phrase and its object; equipe's news as event lines, each
//! of a kind; how long ago a turn ended; what a command under way runs. No
//! UI types here, so the rules are tested without a window.

use equipe_core::i18n::{Lang, core};
use equipe_core::protocol::watch::{CommandWatch, WatchState};

use crate::convo::{Status, ToolLine};

/// More news lines in a row than this fold into one line that counts them.
pub const NEWS_FOLDS_PAST: usize = 3;

/// A step's verb phrase, done or under way: `Ran` / `Running`. A tool
/// without a phrase of its own reads as its name.
/// A line's verb phrase: a command left in the background says so.
pub fn line_phrase(line: &ToolLine, running: bool) -> String {
    if line.background {
        let key = if running { "chat.doing_background" } else { "chat.did_background" };
        return crate::i18n::catalog().get(key).to_owned();
    }
    phrase(&line.verb, running)
}

pub fn phrase(verb: &str, running: bool) -> String {
    let key = match (verb, running) {
        ("run", false) => "chat.did_run",
        ("run", true) => "chat.doing_run",
        ("read", false) => "chat.did_read",
        ("read", true) => "chat.doing_read",
        ("edit", false) => "chat.did_edit",
        ("edit", true) => "chat.doing_edit",
        ("write", false) => "chat.did_write",
        ("write", true) => "chat.doing_write",
        ("search", false) => "chat.did_search",
        ("search", true) => "chat.doing_search",
        ("fetch", false) => "chat.did_fetch",
        ("fetch", true) => "chat.doing_fetch",
        ("delegate", false) => "chat.did_delegate",
        ("delegate", true) => "chat.doing_delegate",
        ("board", false) => "chat.did_board",
        ("board", true) => "chat.doing_board",
        ("ticket", false) => "chat.did_ticket",
        ("ticket", true) => "chat.doing_ticket",
        ("comment", false) => "chat.did_comment",
        ("comment", true) => "chat.doing_comment",
        ("hand in", false) => "chat.did_hand_in",
        ("hand in", true) => "chat.doing_hand_in",
        ("wait on you", false) => "chat.did_wait_on_you",
        ("wait on you", true) => "chat.doing_wait_on_you",
        ("ask you", false) => "chat.did_ask_you",
        ("ask you", true) => "chat.doing_ask_you",
        ("browse", false) => "chat.did_browse",
        ("browse", true) => "chat.doing_browse",
        ("recall", false) => "chat.did_recall",
        ("recall", true) => "chat.doing_recall",
        (other, _) => return pretty_tool(other),
    };
    crate::i18n::catalog().get(key).to_owned()
}

/// `mcp__linear__create_issue` → `linear create_issue`: a tool's name as a
/// person reads it.
fn pretty_tool(name: &str) -> String {
    match name.strip_prefix("mcp__") {
        Some(rest) => rest.replacen("__", " ", 1),
        None => name.to_owned(),
    }
}

/// What a group's summary counts, in the order it names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Count {
    Run,
    Read,
    Edit,
    Search,
    Fetch,
    Browse,
    Delegate,
    Board,
    Recall,
    Other,
}

impl Count {
    fn of(verb: &str) -> Self {
        match verb {
            "run" => Count::Run,
            "read" => Count::Read,
            "edit" | "write" => Count::Edit,
            "search" => Count::Search,
            "fetch" => Count::Fetch,
            "browse" => Count::Browse,
            "delegate" => Count::Delegate,
            "board" | "ticket" | "comment" | "hand in" | "wait on you" | "ask you" => Count::Board,
            "recall" => Count::Recall,
            _ => Count::Other,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Count::Run => "chat.sum_run",
            Count::Read => "chat.sum_read",
            Count::Edit => "chat.sum_edit",
            Count::Search => "chat.sum_search",
            Count::Fetch => "chat.sum_fetch",
            Count::Browse => "chat.sum_browse",
            Count::Delegate => "chat.sum_delegate",
            Count::Board => "chat.sum_board",
            Count::Recall => "chat.sum_recall",
            Count::Other => "chat.sum_other",
        }
    }

    /// Lines that look around count what they looked at; the others, the
    /// calls (an edit, the files it changed).
    fn counts_targets(self) -> bool {
        matches!(self, Count::Read | Count::Search | Count::Fetch | Count::Recall | Count::Edit)
    }
}

/// `Ran 3 commands · Read 2 files`: what a group did, counted, in the order it
/// first did each.
pub fn summary(lines: &[ToolLine]) -> String {
    let mut counts: Vec<(Count, Vec<&str>, usize)> = Vec::new();
    for line in lines {
        let kind = Count::of(&line.verb);
        let at = match counts.iter().position(|(k, _, _)| *k == kind) {
            Some(at) => at,
            None => {
                counts.push((kind, Vec::new(), 0));
                counts.len() - 1
            }
        };
        let (_, targets, calls) = &mut counts[at];
        *calls += 1;
        for target in &line.targets {
            if !targets.contains(&target.as_str()) {
                targets.push(target);
            }
        }
    }
    counts
        .into_iter()
        .map(|(kind, targets, calls)| {
            let n = if kind.counts_targets() && !targets.is_empty() { targets.len() } else { calls };
            crate::i18n::catalog().fill(kind.key(), &[("count", n.to_string())])
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How a group's line reads, closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Title {
    /// One call: its verb phrase and what it acted on.
    Call { phrase: String, object: String },
    /// Several: what they did, counted.
    Summary(String),
}

/// A group's line: the call under way while one runs; else the one call it
/// holds, or a delegated task's own description, or the count.
pub fn title(lines: &[ToolLine]) -> Title {
    if let Some(running) = lines.iter().rev().find(|l| l.status == Status::Running) {
        return Title::Call { phrase: line_phrase(running, true), object: running.text() };
    }
    match lines {
        [one] if one.targets.len() <= 1 => Title::Call { phrase: line_phrase(one, false), object: one.text() },
        _ => match lines.iter().find(|l| l.verb == "delegate" && !l.targets.is_empty()) {
            Some(task) if lines.iter().all(|l| l.verb == "delegate") => Title::Call { phrase: phrase("delegate", false), object: task.text() },
            _ => Title::Summary(summary(lines)),
        },
    }
}

/// Where a turn's activity stands: running while any call runs; otherwise
/// the latest decisive call says whether the turn recovered from an earlier
/// failure. A failed call remains in the opened detail even after recovery.
pub fn status(lines: &[ToolLine]) -> Status {
    if lines.iter().any(|l| l.status == Status::Running) {
        Status::Running
    } else if let Some(last) = lines.iter().rev().find_map(|line| match line.status {
        Status::Done | Status::Failed => Some(line.status),
        Status::Running | Status::Declined | Status::Unknown => None,
    }) {
        last
    } else if lines.iter().any(|l| l.status == Status::Declined) {
        Status::Declined
    } else {
        Status::Unknown
    }
}

/// How long a group took, its calls added up, once none runs.
pub fn total_ms(lines: &[ToolLine]) -> Option<u64> {
    if lines.iter().any(|l| l.status == Status::Running) {
        return None;
    }
    let known: Vec<u64> = lines.iter().filter_map(|l| l.ms).collect();
    (!known.is_empty()).then(|| known.iter().sum())
}

/// A turn's calls stay one activity. Failures remain on their individual
/// detail rows; whether the activity itself failed is decided by `status`.
pub fn groups(run: Vec<ToolLine>) -> Vec<Vec<ToolLine>> {
    if run.is_empty() { Vec::new() } else { vec![run] }
}

/// What a call shows when its row is opened: a command's full text and
/// output, an edit's change, the whole of a long object.
pub fn has_more(line: &ToolLine) -> bool {
    // A described command: the command itself is under its description.
    (line.label.is_some() && line.full.is_some()) || line.output.is_some() || line.diff.is_some() || line.full.as_ref().is_some_and(|f| f.chars().count() > 80 || f.contains('\n'))
}

// ── news ──

/// What a line of equipe's news is about; each has its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewsKind {
    HandedIn,
    Accepted,
    Merged,
    Proposed,
    Waiting,
    Cancelled,
    Reopened,
    Resumed,
    Left,
    SentBack,
    CannotStart,
    DeskSaved,
    YouTold,
    Stale,
    /// The disk ran short, or a build failed for want of room: "Clean up" beside it.
    DiskShort,
    /// The disk has room again.
    DiskRoom,
}

/// A line of news, worded in the language in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct News {
    pub kind: NewsKind,
    pub text: String,
}

/// The news the daemon words (equipe core's `news.*`), and what each is.
const NEWS: &[(&str, NewsKind)] = &[
    ("news.design_qa_asked", NewsKind::HandedIn),
    ("news.design_qa_passed", NewsKind::HandedIn),
    ("news.design_qa_changes", NewsKind::SentBack),
    ("news.handed_in", NewsKind::HandedIn),
    ("news.accepted_home", NewsKind::Accepted),
    ("news.accepted", NewsKind::Accepted),
    ("news.merged_pushed", NewsKind::Merged),
    ("news.merged", NewsKind::Merged),
    ("news.proposed", NewsKind::Proposed),
    ("news.waiting", NewsKind::Waiting),
    ("news.cancelled", NewsKind::Cancelled),
    ("news.reopened_note", NewsKind::Reopened),
    ("news.reopened", NewsKind::Reopened),
    ("news.resumed", NewsKind::Resumed),
    ("news.left", NewsKind::Left),
    ("news.helper_left", NewsKind::Left),
    ("news.sent_back", NewsKind::SentBack),
    ("news.cannot_start", NewsKind::CannotStart),
    ("news.desk_saved", NewsKind::DeskSaved),
    ("news.you_told", NewsKind::YouTold),
    ("news.stale.one", NewsKind::Stale),
    ("news.stale.other", NewsKind::Stale),
    ("news.disk_build_failed", NewsKind::DiskShort),
    ("news.disk_critical", NewsKind::DiskShort),
    ("news.disk_room", NewsKind::DiskRoom),
];

/// A transcript line read as news, if it is: matched against every
/// language's wording (a line kept from before the language changed reads
/// in the one in use now), its marks (`✗`, `✎`, `↪`) left to the icon.
pub fn news_of(text: &str) -> Option<News> {
    let text = text.trim();
    for &(key, kind) in NEWS {
        for wording in Lang::ALL.into_iter().map(|lang| core(lang).get(key)).chain(equipe_core::i18n::previous(key)) {
            let Some(args) = match_template(wording, text) else { continue };
            let base = key.strip_suffix(".one").or_else(|| key.strip_suffix(".other")).unwrap_or(key);
            let worded = core(crate::i18n::lang()).fill(base, &args.iter().map(|(k, v)| (k.as_str(), v.clone())).collect::<Vec<_>>());
            return Some(News { kind, text: unmark(&worded).to_owned() });
        }
    }
    None
}

/// The notices of a member handing the person its browser (equipe core's):
/// into the browser panel, or — no screen with one attached then — onto the
/// daemon's Mac's screen.
const HANDED: &[&str] = &["notice.browser_in_panel", "notice.browser_on_screen"];

/// A transcript line saying a member handed the person its browser, in any
/// language (a line kept from before the language changed, an English one
/// from a daemon that spoke English): who, and what it asked them to do.
pub fn handed_of(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    for key in HANDED {
        for wording in Lang::ALL.into_iter().map(|lang| core(lang).get(key)).chain(equipe_core::i18n::previous(key)) {
            let Some(args) = match_template(wording, text) else { continue };
            let arg = |name: &str| args.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_owned());
            return Some((arg("name")?, arg("why")?));
        }
    }
    None
}

/// A member's report of a flow it tested on a phone (equipe core's
/// `notice.qa_report`, then a line a step, then the key frames as pictures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaReport {
    pub name: String,
    pub flow: String,
    pub ticket: u32,
    pub passed: usize,
    pub total: usize,
    /// The recording, by its path on the Mac that runs the squad.
    pub video: String,
    /// Each step: its verdict, number, what was done and what was seen.
    pub steps: Vec<(equipe_core::phone::report::Verdict, usize, String, String)>,
    /// The key frames, by path.
    pub frames: Vec<String>,
}

impl QaReport {
    pub fn verdict(&self) -> equipe_core::phone::report::Verdict {
        use equipe_core::phone::report::Verdict;
        if self.steps.iter().any(|s| s.0 == Verdict::Fail) {
            Verdict::Fail
        } else if self.steps.iter().any(|s| s.0 == Verdict::Blocked) || self.passed < self.total {
            Verdict::Blocked
        } else {
            Verdict::Pass
        }
    }
}

/// A transcript line that is a phone test's report, in any language.
pub fn qa_report_of(text: &str) -> Option<QaReport> {
    let mut lines = text.trim().lines();
    let head = lines.next()?.trim();
    let args = Lang::ALL.into_iter().map(|lang| core(lang).get("notice.qa_report")).find_map(|wording| match_template(wording, head))?;
    let arg = |name: &str| args.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_owned());
    let mut report = QaReport {
        name: arg("name")?,
        flow: arg("flow")?,
        ticket: arg("ticket")?.parse().ok()?,
        passed: arg("passed")?.parse().ok()?,
        total: arg("steps")?.parse().ok()?,
        video: arg("video")?,
        steps: Vec::new(),
        frames: Vec::new(),
    };
    let rest: Vec<&str> = lines.collect();
    report.steps = rest.iter().filter_map(|l| equipe_core::phone::report::step_line(l)).collect();
    report.frames = equipe_core::visuals::markdown::local_images(&rest.join("\n")).into_iter().map(|i| i.path).filter(|p| p.starts_with('/')).collect();
    Some(report)
}

/// A message equipe sent a member as the person's (before 0.48): its browser
/// handed back to it.
pub fn handed_back_line(text: &str) -> bool {
    let text = text.trim();
    text == "The person finished in the browser."
        || text.starts_with("The person handled the page that needed them in your browser")
        || text.starts_with("The person finished in your browser")
}

/// A line's leading mark and the stale notice's rules, gone: the icon says it.
fn unmark(text: &str) -> &str {
    let text = text.trim().trim_start_matches(['✗', '✎', '↪']).trim_start();
    text.trim_start_matches('─').trim_end_matches('─').trim()
}

/// `{name}`s of `template` as they are in `text`, when the rest is the same.
fn match_template(template: &str, text: &str) -> Option<Vec<(String, String)>> {
    // A Korean particle after a placeholder reads as the one its value took.
    equipe_core::i18n::particle_forms(template).iter().find_map(|form| match_form(form, text))
}

fn match_form(template: &str, text: &str) -> Option<Vec<(String, String)>> {
    // The template cut at its placeholders: literal, name, literal, name, …
    let mut literals: Vec<&str> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}')? + open;
        literals.push(&rest[..open]);
        names.push(rest[open + 1..close].to_owned());
        rest = &rest[close + 1..];
    }
    literals.push(rest);
    if names.is_empty() {
        return (template == text).then(Vec::new);
    }
    let mut at = text.strip_prefix(literals[0])?;
    let mut args = Vec::new();
    for (n, name) in names.iter().enumerate() {
        let next = literals[n + 1];
        let last = n + 1 == names.len();
        let value = if last {
            at.strip_suffix(next)?
        } else if next.is_empty() {
            return None;
        } else {
            &at[..at.find(next)?]
        };
        if value.is_empty() || value.contains('\n') {
            return None;
        }
        args.push((name.clone(), value.to_owned()));
        at = &at[value.len()..];
        at = at.strip_prefix(next)?;
        if last && !at.is_empty() {
            return None;
        }
    }
    // A number stands where the template has one.
    args.iter().all(|(name, value)| !matches!(name.as_str(), "id" | "count") || value.chars().all(|c| c.is_ascii_digit())).then_some(args)
}

/// How a command under way reads under its step, from the daemon's look at
/// what runs for it ([`CommandWatch`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watched {
    /// One quiet line: what runs and how hard (`sqlite3 · 0% CPU · 27m`),
    /// that nothing does, or that it waits on the person.
    Line(String),
    /// It looks stuck: why, in a sentence, and what stopping does. `command`:
    /// there is a command to stop; otherwise only the turn can be.
    Stuck { says: String, hint: String, command: bool },
}

/// What a command's watch shows, for member `name`; `steers`: a message of
/// the person's waits to be read at the next step. Nothing while it is just
/// starting.
pub fn watched(watch: &CommandWatch, name: &str, steers: bool) -> Option<Watched> {
    let words = crate::i18n::catalog();
    let engine = crate::switching::engine_name(watch.engine).to_owned();
    let process = watch.process.as_ref();
    let line = |key: &str| {
        let p = process?;
        Some(words.fill(key, &[("process", p.name.clone()), ("cpu", p.cpu_percent.to_string()), ("time", crate::state::short_span(p.secs))]))
    };
    Some(match watch.state {
        WatchState::Busy => Watched::Line(line("chat.watch_busy")?),
        WatchState::Working => Watched::Line(line("chat.watch_line")?),
        WatchState::Nothing => Watched::Line(words.fill("chat.watch_nothing", &[("engine", engine)])),
        WatchState::AwaitingYou => Watched::Line(words.fill("chat.watch_asks", &[("name", name.to_owned())])),
        WatchState::Stuck { idle_secs } => {
            let (idle, quiet) = (crate::state::short_span(idle_secs), crate::state::short_span(watch.quiet_secs));
            let says = match process {
                Some(p) => words.fill("chat.stuck_idle", &[("process", p.name.clone()), ("idle", idle), ("engine", engine), ("quiet", quiet)]),
                None => words.fill("chat.stuck_nothing", &[("idle", idle), ("engine", engine), ("quiet", quiet)]),
            };
            let hint = match (process.is_some(), steers) {
                (true, true) => words.fill("chat.stuck_steer_hint", &[("name", name.to_owned())]),
                (true, false) => words.fill("chat.stuck_stop_hint", &[("name", name.to_owned())]),
                (false, _) => words.fill("chat.stuck_turn_hint", &[("name", name.to_owned())]),
            };
            Watched::Stuck { says, hint, command: process.is_some() }
        }
    })
}

/// What "Send now" says it did, for member `name`, as the daemon will do it:
/// never stopping the work. At work (`working`): a command running now
/// (`commanding`) is stopped so the next step comes at once; else the member
/// reads it at that step — or, on an engine that can't read mid-work
/// (`steers` false), when the turn ends. Not at work: sent, `count` together.
pub fn read_now(name: &str, working: bool, steers: bool, commanding: bool, count: usize) -> String {
    let words = crate::i18n::catalog();
    let name = [("name", name.to_owned())];
    match (working, steers, commanding) {
        (false, _, _) if count > 1 => words.fill("composer.sent_now_many", &[("count", count.to_string())]),
        (false, _, _) => words.get("composer.sent_now").to_owned(),
        (true, false, _) => words.fill("composer.read_now_later", &name),
        (true, true, true) => words.fill("composer.read_now_command", &name),
        (true, true, false) => words.fill("composer.read_now_next", &name),
    }
}

/// `just now`, `5m ago`, `3h ago`, `2d ago`.
pub fn ago(secs: u64) -> String {
    let words = crate::i18n::catalog();
    match secs {
        0..60 => words.get("chat.ago_now").to_owned(),
        60..3600 => words.fill("chat.ago_mins", &[("count", (secs / 60).to_string())]),
        3600..86_400 => words.fill("chat.ago_hours", &[("count", (secs / 3600).to_string())]),
        _ => words.fill("chat.ago_days", &[("count", (secs / 86_400).to_string())]),
    }
}

/// When a message was said, as the conversation shows it under it: how long
/// ago within the hour (`Just now`, `5 minutes ago`), then the hours today,
/// `Yesterday`, its weekday within the week, then its date — on the person's
/// clock, in their words.
pub fn said_when(at: u64, now: u64) -> String {
    said_label(crate::i18n::catalog(), &SystemZone, at, now)
}

/// The same moment in full, for its tooltip: `Nov 4, 2026, 12:12 PM`.
pub fn said_on(at: u64) -> String {
    let words = crate::i18n::catalog();
    said_full(words, &SystemZone, Hours::of(words.lang, crate::state::system_hours()), at)
}

/// When [`said_when`] next reads differently, in unix seconds.
pub fn said_when_changes(at: u64, now: u64) -> u64 {
    said_changes(&SystemZone, at, now)
}

/// A moment as the person's clock reads it where they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

impl Local {
    /// The date and time `secs` after the epoch reads on a clock at UTC.
    #[cfg(test)]
    pub fn of_utc(secs: i64) -> Self {
        let days = secs.div_euclid(86_400);
        let of_day = secs.rem_euclid(86_400);
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self { year, month, day, weekday: (days + 4).rem_euclid(7) as u32, hour: (of_day / 3600) as u32, minute: (of_day % 3600 / 60) as u32 }
    }

    /// Days from the epoch to its date, whatever its time of day.
    fn day_number(&self) -> i64 {
        let y = self.year - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

/// Where the person is: what a moment reads on their clock, and when their
/// next day begins.
pub trait Zone {
    fn local(&self, unix: u64) -> Local;
    /// The first second of the day after the one `unix` falls in.
    fn next_day(&self, unix: u64) -> u64;
}

/// This Mac's time zone, daylight saving and all.
pub struct SystemZone;

impl Zone for SystemZone {
    fn local(&self, unix: u64) -> Local {
        crate::state::local_time(unix)
    }

    fn next_day(&self, unix: u64) -> u64 {
        // A day is 23 to 25 hours long: 25 past its start is in the next.
        crate::state::local_midnight(crate::state::local_midnight(unix) + 25 * 3600)
    }
}

/// How the person's clock tells the hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hours {
    /// `12:12 PM`.
    Twelve,
    /// `12:12`.
    TwentyFour,
}

impl Hours {
    /// The Mac's clock (`mac`, when it says), else the language's own.
    pub fn of(lang: Lang, mac: Option<Hours>) -> Self {
        mac.unwrap_or(match lang {
            Lang::En | Lang::Ko | Lang::ZhHant => Hours::Twelve,
            Lang::Ja | Lang::ZhHans | Lang::De | Lang::Fr | Lang::Es => Hours::TwentyFour,
        })
    }
}

/// `text` with its first letter a capital: a label begins as a sentence does.
fn capitalized(text: String) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().chain(chars).collect(),
        _ => text,
    }
}

/// The `n`th of a `|`-separated list of words (months, weekdays).
fn nth(list: &str, n: u32) -> String {
    list.split('|').nth(n as usize).unwrap_or_default().to_owned()
}

/// [`said_when`] in `words`, on `zone`'s clock.
pub fn said_label(words: &equipe_core::i18n::Catalog, zone: &dyn Zone, at: u64, now: u64) -> String {
    let age = now.saturating_sub(at);
    if age < 3600 {
        let said = if age < 60 { words.get("chat.ago_now").to_owned() } else { words.fill("chat.ago_mins", &[("count", (age / 60).to_string())]) };
        return capitalized(said);
    }
    let (then, today) = (zone.local(at), zone.local(now));
    match today.day_number() - then.day_number() {
        ..=0 => capitalized(words.fill("chat.ago_hours", &[("count", (age / 3600).to_string())])),
        1 => words.get("time.yesterday").to_owned(),
        2..=6 => nth(words.get("time.weekdays"), then.weekday),
        _ => date(words, &then, then.year != today.year),
    }
}

/// The date of `then`: `Nov 4`, with its year `Nov 4, 2025`.
fn date(words: &equipe_core::i18n::Catalog, then: &Local, with_year: bool) -> String {
    let parts = [("month", nth(words.get("time.months"), then.month - 1)), ("day", then.day.to_string()), ("year", then.year.to_string())];
    equipe_core::i18n::fill(words.get(if with_year { "time.date_year" } else { "time.date" }), &parts)
}

/// [`said_on`] in `words`, on `zone`'s clock telling the hour as `hours` does.
pub fn said_full(words: &equipe_core::i18n::Catalog, zone: &dyn Zone, hours: Hours, at: u64) -> String {
    let then = zone.local(at);
    let mm = format!("{:02}", then.minute);
    let time = match hours {
        Hours::Twelve => {
            let ampm = words.get(if then.hour < 12 { "time.am" } else { "time.pm" }).to_owned();
            let h = match then.hour % 12 {
                0 => 12,
                h => h,
            };
            equipe_core::i18n::fill(words.get("time.clock_12"), &[("h", h.to_string()), ("mm", mm), ("ampm", ampm)])
        }
        Hours::TwentyFour => equipe_core::i18n::fill(words.get("time.clock_24"), &[("hh", format!("{:02}", then.hour)), ("mm", mm)]),
    };
    equipe_core::i18n::fill(words.get("time.full"), &[("date", date(words, &then, true)), ("time", time)])
}

/// When [`said_label`] next reads differently: at the next minute within the
/// hour, the next hour today, else the next midnight (a weekday or a date
/// may read the same after it — asked again then, once a day).
pub fn said_changes(zone: &dyn Zone, at: u64, now: u64) -> u64 {
    let age = now.saturating_sub(at);
    if age < 3600 {
        return at + (age / 60 + 1) * 60;
    }
    let next_day = zone.next_day(now);
    if zone.local(at).day_number() == zone.local(now).day_number() { (at + (age / 3600 + 1) * 3600).min(next_day) } else { next_day }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_report_reads_back_as_a_card_in_any_language() {
        use equipe_core::phone::report::{Report, Step, Verdict, head_args, note};
        let report = Report {
            flow: "상품 두 개 결제".into(),
            ticket: 12,
            member: "qa".into(),
            recording: "/h/recordings/T-12/checkout.mp4".into(),
            seconds: Some(6.),
            steps: vec![
                Step { step: "Add item 두 번 탭".into(), expected: "Items: 2".into(), seen: "Items: 2".into(), verdict: Verdict::Pass, frame: Some("/h/recordings/T-12/checkout/02.jpg".into()) },
                Step { step: "Checkout 탭".into(), expected: "Total: $10".into(), seen: "Total: $5".into(), verdict: Verdict::Fail, frame: Some("/h/recordings/T-12/checkout/05.jpg".into()) },
            ],
        };
        for lang in [Lang::Ko, Lang::En, Lang::Ja, Lang::De] {
            let head = core(lang).fill("notice.qa_report", &head_args(&report));
            let card = qa_report_of(&note(&head, &report)).unwrap_or_else(|| panic!("{lang:?}: {head}"));
            assert_eq!((card.name.as_str(), card.flow.as_str(), card.ticket, card.passed, card.total), ("qa", "상품 두 개 결제", 12, 1, 2), "{lang:?}");
            assert_eq!(card.video, "/h/recordings/T-12/checkout.mp4");
            assert_eq!(card.steps[1], (Verdict::Fail, 2, "Checkout 탭".to_owned(), "Total: $5".to_owned()));
            assert_eq!(card.frames, ["/h/recordings/T-12/checkout/02.jpg", "/h/recordings/T-12/checkout/05.jpg"]);
            assert_eq!(card.verdict(), Verdict::Fail);
        }
        assert!(qa_report_of("frontend handed in T-4").is_none());
    }

    fn line(verb: &str, targets: &[&str], status: Status) -> ToolLine {
        let mut l = ToolLine::new(verb, targets.iter().map(|t| (*t).to_owned()).collect(), matches!(verb, "edit" | "write"));
        l.status = status;
        l
    }

    #[test]
    fn a_summary_counts_what_the_group_did_in_the_order_it_did_it() {
        let lines = [
            line("run", &["cargo test"], Status::Done),
            line("read", &["a.rs", "b.rs"], Status::Done),
            line("run", &["git status"], Status::Done),
            line("read", &["a.rs"], Status::Done),
            line("edit", &["a.rs"], Status::Done),
            line("edit", &["a.rs"], Status::Done),
        ];
        assert_eq!(summary(&lines), "명령 2개 실행 · 파일 2개 읽음 · 파일 1개 수정", "a file read or edited twice counts once");
        let board = [line("comment", &["T-118 · PM: 확인"], Status::Done), line("hand in", &["T-118"], Status::Done), line("mcp__linear__x", &[], Status::Done)];
        assert_eq!(summary(&board), "보드 작업 2개 · 도구 1번 사용");
    }

    #[test]
    fn summaries_take_each_languages_plural() {
        let words = equipe_core::i18n::Catalog::load(Lang::En, crate::i18n::FILES);
        assert_eq!(words.fill("chat.sum_run", &[("count", "1".into())]), "Ran 1 command");
        assert_eq!(words.fill("chat.sum_run", &[("count", "3".into())]), "Ran 3 commands");
        let de = equipe_core::i18n::Catalog::load(Lang::De, crate::i18n::FILES);
        assert_ne!(de.fill("chat.sum_read", &[("count", "1".into())]), de.fill("chat.sum_read", &[("count", "2".into())]).replace('2', "1"));
        for lang in Lang::ALL {
            let words = equipe_core::i18n::Catalog::load(lang, crate::i18n::FILES);
            for key in ["chat.activity", "chat.sum_run", "chat.sum_read", "chat.sum_edit", "chat.news_count", "chat.ago_mins"] {
                let said = words.fill(key, &[("count", "4".into())]);
                assert!(said.contains('4') && !said.contains('{'), "{lang:?} {key}: {said}");
            }
        }
    }

    #[test]
    fn a_group_reads_as_its_one_call_or_its_count_and_runs_live() {
        let one = [line("run", &["ps -axo pid"], Status::Done)];
        assert_eq!(title(&one), Title::Call { phrase: "실행함".into(), object: "ps -axo pid".into() });
        let many = [line("run", &["ls"], Status::Done), line("read", &["a.rs"], Status::Done)];
        assert_eq!(title(&many), Title::Summary("명령 1개 실행 · 파일 1개 읽음".into()));
        // A merged read of three files is a count, not three names.
        assert_eq!(title(&[line("read", &["a", "b", "c"], Status::Done)]), Title::Summary("파일 3개 읽음".into()));
        // While a call runs the line names it, in the present.
        let live = [line("read", &["a.rs"], Status::Done), line("run", &["cargo test"], Status::Running)];
        assert_eq!(title(&live), Title::Call { phrase: "실행 중".into(), object: "cargo test".into() });
        assert_eq!(status(&live), Status::Running);
        assert_eq!(total_ms(&live), None, "no total while it runs");
        // A delegated task reads as what it was for.
        assert_eq!(title(&[line("delegate", &["사이드바 접기 위치 고치기"], Status::Done)]), Title::Call { phrase: "에이전트 실행함".into(), object: "사이드바 접기 위치 고치기".into() });
    }

    #[test]
    fn a_failed_call_stays_in_the_activity_and_a_later_success_recovers_it() {
        let run = vec![line("read", &["a"], Status::Done), line("run", &["cargo test"], Status::Failed), line("run", &["cargo fix"], Status::Done), line("edit", &["a"], Status::Done)];
        let cut = groups(run);
        assert_eq!(cut.iter().map(Vec::len).collect::<Vec<_>>(), [4]);
        assert_eq!(status(&cut[0]), Status::Done, "the failure remains in detail, but the later successful path decides the activity");
        assert_eq!(status(&[line("read", &["a"], Status::Done), line("run", &["cargo test"], Status::Failed)]), Status::Failed, "a final failure stays visible");
        assert_eq!(groups(vec![]).len(), 0);
    }

    #[test]
    fn times_add_up_over_the_calls_that_say() {
        let mut a = line("run", &["a"], Status::Done);
        a.ms = Some(300);
        let mut b = line("run", &["b"], Status::Done);
        b.ms = Some(1200);
        assert_eq!(total_ms(&[a, b, line("read", &["c"], Status::Done)]), Some(1500));
        assert_eq!(total_ms(&[line("read", &["c"], Status::Unknown)]), None);
    }

    #[test]
    fn news_is_known_in_any_language_and_reads_in_the_one_in_use() {
        let old = news_of("backend handed in T-118 for your review · 주문 API.").unwrap();
        assert_eq!(old, News { kind: NewsKind::HandedIn, text: "backend가 T-118을 검토해 달라고 넘겼어요 · 주문 API.".into() });
        assert_eq!(news_of("You accepted T-58 · 헤더.").unwrap().kind, NewsKind::Accepted);
        assert_eq!(news_of("pm이(가) T-9을(를) 제안했습니다: 로그인").unwrap().kind, NewsKind::Proposed);
        // Its particles as the names took them, or as kept from before they did.
        let handed = news_of("backend가 T-118을 검토해 달라고 넘겼어요 · 주문 API.").unwrap();
        assert_eq!((handed.kind, handed.text.as_str()), (NewsKind::HandedIn, "backend가 T-118을 검토해 달라고 넘겼어요 · 주문 API."));
        assert_eq!(news_of("backend이(가) T-118을(를) 검토해 달라고 넘겼어요 · 주문 API.").unwrap().kind, NewsKind::HandedIn);
        assert_eq!(news_of("mobile이 T-7을 검토해 달라고 넘겼어요 · 앱.").unwrap().kind, NewsKind::HandedIn);
        let failed = news_of("✗ T-4 couldn't start for designer: no workspace").unwrap();
        assert_eq!(failed.kind, NewsKind::CannotStart);
        assert!(!failed.text.starts_with('✗'), "the icon says it: {}", failed.text);
        assert_eq!(news_of("── 3 tickets haven't moved in a week: the PM will suggest tidying them ──").unwrap().kind, NewsKind::Stale);
        assert_eq!(news_of("── T-4 · Research"), None, "a ticket's heading is not news");
        assert_eq!(news_of("plain words"), None);
        assert_eq!(news_of("You accepted T-x · 헤더."), None, "a number where the number is");
    }

    fn watch(process: Option<(&str, u32)>, state: WatchState) -> CommandWatch {
        use equipe_core::protocol::watch::WatchedProcess;
        let process = process.map(|(name, cpu_percent)| WatchedProcess { name: name.into(), cpu_percent, secs: 27 * 60 + 5 });
        CommandWatch { engine: equipe_core::protocol::Engine::Claude, process, quiet_secs: 28 * 60 + 6, state }
    }

    #[test]
    fn a_command_under_way_reads_as_what_runs_and_how_hard() {
        assert_eq!(watched(&watch(Some(("sqlite3", 0)), WatchState::Working), "pm", false), Some(Watched::Line("sqlite3 · CPU 0% · 27분".into())));
        assert_eq!(watched(&watch(Some(("cargo", 640)), WatchState::Busy), "pm", false), Some(Watched::Line("cargo 작업 중 · CPU 640% · 27분".into())));
        assert_eq!(watched(&watch(None, WatchState::Nothing), "pm", false), Some(Watched::Line("Claude 응답 대기 중 — 실행 중인 것 없음".into())));
        assert_eq!(watched(&watch(None, WatchState::AwaitingYou), "pm", false), Some(Watched::Line("pm의 요청에 대한 답을 기다리는 중".into())));
        assert_eq!(watched(&watch(None, WatchState::Working), "pm", false), None, "just starting: nothing to say yet");
    }

    #[test]
    fn a_stuck_command_says_why_and_what_stopping_it_does() {
        let Some(Watched::Stuck { says, hint, command: true }) = watched(&watch(Some(("sqlite3", 0)), WatchState::Stuck { idle_secs: 27 * 60 }), "pm", true) else { panic!() };
        assert_eq!(says, "sqlite3 프로세스가 27분 동안 CPU를 거의 쓰지 않았고, Claude도 28분 동안 아무 말이 없어요.");
        assert!(hint.contains("오류를 받고") && hint.contains("메시지도"), "a message waiting is read once it stops: {hint}");
        let Some(Watched::Stuck { says, hint, command: false }) = watched(&watch(None, WatchState::Stuck { idle_secs: 600 }), "pm", false) else { panic!() };
        assert!(says.starts_with("이 명령으로 10분 동안"), "{says}");
        assert!(hint.contains("작업을 멈추면"), "only the turn can be stopped: {hint}");
    }

    #[test]
    fn send_now_never_says_the_work_was_stopped() {
        assert_eq!(read_now("pm", true, true, true, 1), "명령을 멈추는 중 — pm이 바로 읽고 어떻게 할지 정해요");
        assert_eq!(read_now("pm", true, true, false, 2), "pm이 곧 다음 단계에서 읽어요 — 하던 일은 계속돼요");
        assert!(read_now("pm", true, false, false, 1).contains("이번 차례가 끝나면"), "an engine that can't read mid-work: at the turn's end");
        assert_eq!(read_now("pm", false, true, false, 1), "지금 보냈어요");
        assert_eq!(read_now("pm", false, true, false, 3), "기다리던 3개를 한 메시지로 지금 보냈어요");
        for flash in [read_now("pm", true, true, true, 1), read_now("pm", true, true, false, 1), read_now("pm", true, false, false, 1), read_now("pm", false, true, false, 1)] {
            assert!(!flash.contains("멈췄어요"), "{flash}");
        }
    }

    #[test]
    fn ago_reads_short() {
        assert_eq!(ago(10), "방금");
        assert_eq!(ago(15 * 60), "15분 전");
        assert_eq!(ago(3 * 3600 + 5), "3시간 전");
        assert_eq!(ago(2 * 86_400), "2일 전");
    }

    mod said {
        use super::super::*;

        /// A place `hours` from UTC, all year round.
        struct Offset(i64);

        impl Zone for Offset {
            fn local(&self, unix: u64) -> Local {
                Local::of_utc(unix as i64 + self.0 * 3600)
            }

            fn next_day(&self, unix: u64) -> u64 {
                let local = unix as i64 + self.0 * 3600;
                ((local.div_euclid(86_400) + 1) * 86_400 - self.0 * 3600) as u64
            }
        }

        const UTC: Offset = Offset(0);
        /// Fri 2026-11-06 15:00 UTC.
        const NOW: u64 = 1_793_977_200;
        /// Wed 2026-11-04 12:12 UTC.
        const WED: u64 = 1_793_794_320;
        /// Thu 2026-11-05 23:59 UTC, and the next day's first second.
        const LATE: u64 = 1_793_923_140;
        const MIDNIGHT: u64 = 1_793_923_200;

        fn label(lang: Lang, zone: &dyn Zone, at: u64, now: u64) -> String {
            said_label(crate::i18n::of(lang), zone, at, now)
        }

        fn full(lang: Lang, zone: &dyn Zone, hours: Hours, at: u64) -> String {
            said_full(crate::i18n::of(lang), zone, hours, at)
        }

        #[test]
        fn a_message_reads_how_long_ago_today_then_yesterday_then_its_weekday_then_its_date() {
            let en = |at| label(Lang::En, &UTC, at, NOW);
            assert_eq!(en(NOW), "Just now");
            assert_eq!(en(NOW - 59), "Just now");
            assert_eq!(en(NOW + 30), "Just now", "a clock a little ahead: not in the future");
            assert_eq!(en(NOW - 60), "1 minute ago");
            assert_eq!(en(NOW - 5 * 60 - 30), "5 minutes ago");
            assert_eq!(en(NOW - 3 * 3600), "3 hours ago");
            assert_eq!(en(MIDNIGHT), "15 hours ago", "today's first second is still today");
            assert_eq!(en(LATE), "Yesterday", "a minute before midnight is yesterday");
            assert_eq!(en(WED), "Wednesday");
            assert_eq!(en(1_793_520_000), "Sunday", "five days back: its weekday");
            assert_eq!(en(1_793_347_200), "Oct 30", "a week back: its date");
            assert_eq!(en(1_767_171_900), "Dec 31, 2025", "another year: with it");

            let ko = |at| label(Lang::Ko, &UTC, at, NOW);
            assert_eq!(ko(NOW - 10), "방금");
            assert_eq!(ko(NOW - 15 * 60), "15분 전");
            assert_eq!(ko(NOW - 3 * 3600), "3시간 전");
            assert_eq!(ko(LATE), "어제");
            assert_eq!(ko(WED), "수요일");
            assert_eq!(ko(1_793_347_200), "10월 30일");
            assert_eq!(ko(1_767_171_900), "2025년 12월 31일");
        }

        #[test]
        fn midnight_is_where_the_person_is() {
            // 23:59 on Thursday in UTC is already Friday morning in Seoul and
            // still Thursday afternoon in New York.
            let seoul = Offset(9);
            let new_york = Offset(-5);
            let next_morning = MIDNIGHT + 10 * 3600;
            assert_eq!(label(Lang::En, &UTC, LATE, next_morning), "Yesterday");
            assert_eq!(label(Lang::En, &seoul, LATE, next_morning), "10 hours ago", "Fri 08:59 and Fri 19:00 in Seoul");
            assert_eq!(label(Lang::En, &new_york, LATE, next_morning), "Yesterday", "Thu 18:59 and Fri 05:00 there");
            assert_eq!(label(Lang::En, &new_york, LATE, MIDNIGHT + 4 * 3600), "4 hours ago", "Thu 18:59 and Thu 23:00 there");
            assert_eq!(full(Lang::En, &seoul, Hours::Twelve, LATE), "Nov 6, 2026, 8:59 AM");
            assert_eq!(full(Lang::En, &new_york, Hours::Twelve, LATE), "Nov 5, 2026, 6:59 PM");
        }

        #[test]
        fn the_label_reads_differently_at_the_next_minute_hour_or_midnight() {
            assert_eq!(said_changes(&UTC, NOW - 10, NOW), NOW + 50, "just now, for its first minute");
            assert_eq!(said_changes(&UTC, NOW - 90, NOW), NOW + 30, "1 minute ago until it has been 2");
            assert_eq!(said_changes(&UTC, NOW - 3 * 3600 - 10, NOW), NOW + 3600 - 10, "3 hours ago until 4");
            assert_eq!(said_changes(&UTC, NOW - 14 * 3600, NOW), NOW + 3600, "14 hours ago until 15");
            assert_eq!(said_changes(&UTC, MIDNIGHT + 60, MIDNIGHT + 86_400 - 1_800), MIDNIGHT + 86_400, "23 hours ago becomes yesterday at midnight, not at 24");
            assert_eq!(said_changes(&UTC, LATE, NOW), MIDNIGHT + 86_400, "yesterday becomes its weekday at midnight");
            assert_eq!(said_changes(&UTC, 1_767_171_900, NOW), MIDNIGHT + 86_400);
            // Past each moment it names, it says something else.
            for at in [NOW - 10, NOW - 90, NOW - 3 * 3600 - 10, NOW - 14 * 3600, LATE] {
                let next = said_changes(&UTC, at, NOW);
                assert_eq!(label(Lang::En, &UTC, at, next - 1), label(Lang::En, &UTC, at, NOW), "{at}: the same until then");
                assert_ne!(label(Lang::En, &UTC, at, next), label(Lang::En, &UTC, at, NOW), "{at}: different from then");
            }
        }

        #[test]
        fn the_full_time_is_in_the_persons_words_and_clock() {
            let at = WED;
            let cases = [
                (Lang::En, Hours::Twelve, "Nov 4, 2026, 12:12 PM"),
                (Lang::En, Hours::TwentyFour, "Nov 4, 2026, 12:12"),
                (Lang::Ko, Hours::Twelve, "2026년 11월 4일 오후 12:12"),
                (Lang::Ko, Hours::TwentyFour, "2026년 11월 4일 12:12"),
                (Lang::Ja, Hours::TwentyFour, "2026年11月4日 12:12"),
                (Lang::Ja, Hours::Twelve, "2026年11月4日 午後12:12"),
                (Lang::ZhHans, Hours::TwentyFour, "2026年11月4日 12:12"),
                (Lang::ZhHant, Hours::Twelve, "2026年11月4日 下午12:12"),
                (Lang::De, Hours::TwentyFour, "4. Nov. 2026, 12:12"),
                (Lang::Fr, Hours::TwentyFour, "4 nov. 2026 à 12:12"),
                (Lang::Es, Hours::TwentyFour, "4 nov 2026, 12:12"),
                (Lang::Es, Hours::Twelve, "4 nov 2026, 12:12 p. m."),
            ];
            for (lang, hours, want) in cases {
                assert_eq!(full(lang, &UTC, hours, at), want, "{lang:?} {hours:?}");
            }
            // Midnight and noon on a 12-hour clock, and an hour with one digit on a 24-hour one.
            assert_eq!(full(Lang::En, &UTC, Hours::Twelve, MIDNIGHT), "Nov 6, 2026, 12:00 AM");
            assert_eq!(full(Lang::Ko, &UTC, Hours::Twelve, MIDNIGHT + 9 * 3600 + 5 * 60), "2026년 11월 6일 오전 9:05");
            assert_eq!(full(Lang::De, &UTC, Hours::TwentyFour, MIDNIGHT + 9 * 3600 + 5 * 60), "6. Nov. 2026, 09:05");
        }

        #[test]
        fn each_language_has_its_own_clock_unless_the_mac_says_otherwise() {
            for (lang, hours) in [(Lang::En, Hours::Twelve), (Lang::Ko, Hours::Twelve), (Lang::ZhHant, Hours::Twelve), (Lang::Ja, Hours::TwentyFour), (Lang::ZhHans, Hours::TwentyFour), (Lang::De, Hours::TwentyFour), (Lang::Fr, Hours::TwentyFour), (Lang::Es, Hours::TwentyFour)] {
                assert_eq!(Hours::of(lang, None), hours, "{lang:?}");
                assert_eq!(Hours::of(lang, Some(Hours::TwentyFour)), Hours::TwentyFour, "the Mac set to 24-hour: {lang:?}");
                assert_eq!(Hours::of(lang, Some(Hours::Twelve)), Hours::Twelve);
            }
        }

        #[test]
        fn every_language_says_every_part_of_a_date() {
            for lang in Lang::ALL {
                let words = crate::i18n::of(lang);
                assert_eq!(words.get("time.months").split('|').count(), 12, "{lang:?}");
                assert_eq!(words.get("time.weekdays").split('|').count(), 7, "{lang:?}");
                for at in [NOW, NOW - 5 * 60, NOW - 3 * 3600, LATE, WED, 1_793_347_200, 1_767_171_900] {
                    let said = label(lang, &UTC, at, NOW);
                    assert!(!said.is_empty() && !said.contains('{') && !said.starts_with("time."), "{lang:?}: {said}");
                    assert_eq!(said.chars().next().map(char::is_lowercase), Some(false), "{lang:?}: a label starts as a sentence does: {said}");
                }
                assert!(!full(lang, &UTC, Hours::Twelve, WED).contains('{'), "{lang:?}");
            }
        }
    }

    #[test]
    fn a_tool_without_words_reads_as_its_name() {
        assert_eq!(phrase("mcp__linear__create_issue", false), "linear create_issue");
        assert_eq!(phrase("run", false), "실행함");
    }
}
