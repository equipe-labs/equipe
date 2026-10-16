//! How the conversation's activity reads: tool calls gathered into groups,
//! each group in one line (what it was for, or a count of what it did), each
//! call as a verb phrase and its object; equipe's news as event lines, each
//! of a kind; how long ago a turn ended. No UI types here, so the rules are
//! tested without a window.

use equipe_core::i18n::{Lang, core};

use crate::convo::{Status, ToolLine};

/// More news lines in a row than this fold into one line that counts them.
pub const NEWS_FOLDS_PAST: usize = 3;

/// A step's verb phrase, done or under way: `실행함` / `실행 중`. A tool
/// without a phrase of its own reads as its name.
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

/// `명령 3개 실행 · 파일 2개 읽음`: what a group did, counted, in the order it
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
        return Title::Call { phrase: phrase(&running.verb, true), object: running.text() };
    }
    match lines {
        [one] if one.targets.len() <= 1 => Title::Call { phrase: phrase(&one.verb, false), object: one.text() },
        _ => match lines.iter().find(|l| l.verb == "delegate" && !l.targets.is_empty()) {
            Some(task) if lines.iter().all(|l| l.verb == "delegate") => Title::Call { phrase: phrase("delegate", false), object: task.text() },
            _ => Title::Summary(summary(lines)),
        },
    }
}

/// Where a group stands: running while any call runs, failed when one did.
pub fn status(lines: &[ToolLine]) -> Status {
    if lines.iter().any(|l| l.status == Status::Running) {
        Status::Running
    } else if lines.iter().any(|l| l.status == Status::Failed) {
        Status::Failed
    } else if lines.iter().all(|l| l.status == Status::Done) {
        Status::Done
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

/// A run of calls cut into groups: a call that failed stands on its own,
/// so it stays in sight.
pub fn groups(run: Vec<ToolLine>) -> Vec<Vec<ToolLine>> {
    let mut out: Vec<Vec<ToolLine>> = Vec::new();
    let mut current: Vec<ToolLine> = Vec::new();
    for line in run {
        if line.status == Status::Failed {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(vec![line]);
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// What a call shows when its row is opened: a command's full text and
/// output, an edit's change, the whole of a long object.
pub fn has_more(line: &ToolLine) -> bool {
    line.output.is_some() || line.diff.is_some() || line.full.as_ref().is_some_and(|f| f.chars().count() > 80 || f.contains('\n'))
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
    /// The disk ran short, or a build failed for want of room: 정리하기 beside it.
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
        for lang in Lang::ALL {
            let Some(args) = match_template(core(lang).get(key), text) else { continue };
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
        for lang in Lang::ALL {
            let Some(args) = match_template(core(lang).get(key), text) else { continue };
            let arg = |name: &str| args.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_owned());
            return Some((arg("name")?, arg("why")?));
        }
    }
    None
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

/// `방금`, `5분 전`, `3시간 전`, `2일 전`.
pub fn ago(secs: u64) -> String {
    let words = crate::i18n::catalog();
    match secs {
        0..60 => words.get("chat.ago_now").to_owned(),
        60..3600 => words.fill("chat.ago_mins", &[("count", (secs / 60).to_string())]),
        3600..86_400 => words.fill("chat.ago_hours", &[("count", (secs / 3600).to_string())]),
        _ => words.fill("chat.ago_days", &[("count", (secs / 86_400).to_string())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            for key in ["chat.sum_run", "chat.sum_read", "chat.sum_edit", "chat.news_count", "chat.ago_mins"] {
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
    fn a_failed_call_breaks_its_group_and_stands_alone() {
        let run = vec![line("read", &["a"], Status::Done), line("run", &["cargo test"], Status::Failed), line("run", &["cargo fix"], Status::Done), line("edit", &["a"], Status::Done)];
        let cut = groups(run);
        assert_eq!(cut.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1, 2]);
        assert_eq!(status(&cut[1]), Status::Failed);
        assert_eq!(status(&cut[2]), Status::Done);
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
        assert_eq!(old, News { kind: NewsKind::HandedIn, text: "backend이(가) T-118을(를) 검토해 달라고 넘겼습니다 · 주문 API.".into() });
        assert_eq!(news_of("You accepted T-58 · 헤더.").unwrap().kind, NewsKind::Accepted);
        assert_eq!(news_of("pm이(가) T-9을(를) 제안했습니다: 로그인").unwrap().kind, NewsKind::Proposed);
        let failed = news_of("✗ T-4 couldn't start for designer: no workspace").unwrap();
        assert_eq!(failed.kind, NewsKind::CannotStart);
        assert!(!failed.text.starts_with('✗'), "the icon says it: {}", failed.text);
        assert_eq!(news_of("── 3 tickets haven't moved in a week: the PM will suggest tidying them ──").unwrap().kind, NewsKind::Stale);
        assert_eq!(news_of("── T-4 · Research"), None, "a ticket's heading is not news");
        assert_eq!(news_of("plain words"), None);
        assert_eq!(news_of("You accepted T-x · 헤더."), None, "a number where the number is");
    }

    #[test]
    fn ago_reads_short() {
        assert_eq!(ago(10), "방금");
        assert_eq!(ago(15 * 60), "15분 전");
        assert_eq!(ago(3 * 3600 + 5), "3시간 전");
        assert_eq!(ago(2 * 86_400), "2일 전");
    }

    #[test]
    fn a_tool_without_words_reads_as_its_name() {
        assert_eq!(phrase("mcp__linear__create_issue", false), "linear create_issue");
        assert_eq!(phrase("run", false), "실행함");
    }
}
