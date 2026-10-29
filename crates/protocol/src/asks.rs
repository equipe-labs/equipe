//! What a ticket put down for the person (`block_on_person`) asks of them,
//! item by item, and the one message their answers go back to the member in.
//!
//! The list travels as [`crate::Ticket::asks`]. The ticket's note keeps it too,
//! as a checklist the board and the member read ([`waiting_note`]); a daemon
//! older than the list sends only the note, and a line from before either
//! (`a., b., c.` — the asks once joined with ", ") is split back
//! ([`split_joined`]).

/// The checklist's first line in a waiting ticket's note.
pub const WAITING_HEAD: &str = "Waiting on you:";

/// A waiting ticket's note: the person's checklist, one ask a `- [ ]` line (a
/// line break inside one ask goes on, indented, under it), and why.
pub fn waiting_note(asks: &[String], why: Option<&str>) -> String {
    let mut out = String::from(WAITING_HEAD);
    for ask in asks {
        let mut lines = ask.trim().lines();
        out.push_str(&format!("\n- [ ] {}", lines.next().unwrap_or_default().trim_end()));
        for more in lines {
            out.push_str(&format!("\n  {}", more.trim_end()));
        }
    }
    if let Some(why) = why.map(str::trim).filter(|w| !w.is_empty()) {
        out.push_str(&format!("\n({why})"));
    }
    out
}

/// The asks a waiting ticket's note lists, each whole (its indented lines
/// joined back on).
pub fn waiting_needs(note: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for line in note.lines() {
        if let Some(ask) = line.trim_start().strip_prefix("- [ ] ").or_else(|| line.trim_start().strip_prefix("- [x] ")) {
            out.push(ask.trim().to_owned());
            open = true;
        } else if open && line.starts_with("  ") && !line.trim().is_empty() {
            if let Some(last) = out.last_mut() {
                last.push('\n');
                last.push_str(line.trim());
            }
        } else {
            open = false;
        }
    }
    out.retain(|a| !a.is_empty());
    out
}

/// Why the ticket waits, as its note says it under the checklist: `(…)`.
pub fn waiting_why(note: &str) -> Option<String> {
    if !note.trim_start().starts_with(WAITING_HEAD) {
        return None;
    }
    let last = note.lines().last()?.trim();
    last.strip_prefix('(').and_then(|l| l.strip_suffix(')')).map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned)
}

/// Asks once joined into one line with ", " — each a sentence of its own,
/// so a joint follows the end of one (`.`, `?`, `!`, `。`…): split there,
/// and only there (a comma inside an ask stays).
pub fn split_joined(text: &str) -> Vec<String> {
    const ENDS: &[char] = &['.', '?', '!', '。', '？', '！', '…'];
    let text = text.trim();
    let mut out = Vec::new();
    let mut from = 0;
    for (at, _) in text.match_indices(", ") {
        let before = text[..at].trim_end();
        if before.ends_with(ENDS) && !before[from..].trim().is_empty() {
            out.push(text[from..at].trim().to_owned());
            from = at + 2;
        }
    }
    let rest = text[from..].trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

/// What the person gives back to one ask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Reply {
    /// Nothing yet: it stays open on the ticket.
    #[default]
    Empty,
    /// Their words.
    Text(String),
    /// Done (a thing to do, ticked off).
    Done,
    /// Handed back: the member does it itself ("대신 해 줘").
    Yours,
    /// Not now: the member goes on without it, and it stays open on the ticket.
    Later,
    /// The member's call ("네가 정해 줘").
    YourCall,
    /// The person doesn't know: the member finds another way, or asks differently.
    DontKnow,
}

impl Reply {
    pub fn is_empty(&self) -> bool {
        match self {
            Reply::Empty => true,
            Reply::Text(t) => t.trim().is_empty(),
            _ => false,
        }
    }

    /// Still the person's after it is sent: not yet answered, or put off.
    pub fn stays_open(&self) -> bool {
        self.is_empty() || *self == Reply::Later
    }
}

/// Everything the card holds: a reply and a note to each ask, and the
/// person's own words to the member at the end.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answers {
    pub replies: Vec<Reply>,
    pub notes: Vec<String>,
    pub free: String,
}

impl Answers {
    /// Nothing set: no reply, no note, no words.
    pub fn is_empty(&self) -> bool {
        self.replies.iter().all(Reply::is_empty) && self.notes.iter().all(|n| n.trim().is_empty()) && self.free.trim().is_empty()
    }
}

/// The first line of the message the answers go back in.
pub const ANSWERS_HEAD: &str = "Answers to what you asked for:";
/// What each reply that isn't words says, after the ask.
const DONE: &str = "done ✓";
const YOURS: &str = "do it yourself — the person handed this back to you";
const LATER: &str = "later — go on without it; it stays open on the ticket";
const YOUR_CALL: &str = "your call — decide it yourself and say what you chose";
const DONT_KNOW: &str = "the person doesn't know — find another way, or ask it differently";
const NOT_YET: &str = "— (not yet: still open on the ticket)";
/// Under an ask: the person's note on it.
const NOTE: &str = "Note: ";
/// The person's own words, last.
const FROM: &str = "From the person: ";
/// Lines that go on under an ask.
const INDENT: &str = "   ";

/// Longer than this, an ask is quoted by its start in the answer.
const QUOTED: usize = 80;

/// An ask as the answer quotes it: its first line, cut at [`QUOTED`].
fn quote(ask: &str) -> String {
    let first = ask.lines().next().unwrap_or_default().trim().replace(" → ", " - ");
    if first.chars().count() > QUOTED || ask.trim().lines().count() > 1 {
        format!("{}…", first.chars().take(QUOTED).collect::<String>().trim_end())
    } else {
        first
    }
}

fn said(reply: &Reply) -> String {
    match reply {
        Reply::Text(text) if !text.trim().is_empty() => text.trim().lines().map(str::trim_end).collect::<Vec<_>>().join(&format!("\n{INDENT}")),
        Reply::Done => DONE.to_owned(),
        Reply::Yours => YOURS.to_owned(),
        Reply::Later => LATER.to_owned(),
        Reply::YourCall => YOUR_CALL.to_owned(),
        Reply::DontKnow => DONT_KNOW.to_owned(),
        _ => NOT_YET.to_owned(),
    }
}

/// The one message the person's answers go to the member in: every ask,
/// numbered as it was asked and quoted by its start, with what came back —
/// their words, done, do it yourself, later, your call, don't know, or not
/// yet — a note under it when they wrote one, and their own words last:
///
/// ```text
/// Answers to what you asked for:
/// 1) 심사 연락처를 알려 주세요 → +82 10 1234 5678
/// 2) EU DSA 상태를 trader로 바꿔 주세요 → do it yourself — the person handed this back to you
///    Note: 2FA 코드는 제 폰으로 와요
/// From the person: 이건 너가 직접 해
/// ```
///
/// With nothing asked, their words alone.
pub fn answer_message(asks: &[String], answers: &Answers) -> String {
    let free = answers.free.trim();
    if asks.is_empty() {
        return free.to_owned();
    }
    let mut out = String::from(ANSWERS_HEAD);
    for (n, ask) in asks.iter().enumerate() {
        let reply = answers.replies.get(n).cloned().unwrap_or_default();
        out.push_str(&format!("\n{}) {} → {}", n + 1, quote(ask), said(&reply)));
        if let Some(note) = answers.notes.get(n).map(|n| n.trim()).filter(|n| !n.is_empty()) {
            let note = note.lines().map(str::trim_end).collect::<Vec<_>>().join(&format!("\n{INDENT}"));
            out.push_str(&format!("\n{INDENT}{NOTE}{note}"));
        }
    }
    if !free.is_empty() {
        out.push_str(&format!("\n{FROM}{free}"));
    }
    out
}

/// One ask, as a message of answers carries it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadItem {
    /// Its number as asked, from 1.
    pub n: usize,
    /// The ask as quoted.
    pub ask: String,
    pub reply: Reply,
    pub note: String,
}

/// A message of answers, read back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    pub items: Vec<ReadItem>,
    /// The person's own words.
    pub free: String,
    /// Every ask is in it, answered or not ([`answer_message`]'s); an older
    /// screen's lists only the answered ones.
    pub whole: bool,
}

impl Read {
    /// How many asks it answered (not left open nor put off).
    pub fn answered(&self) -> usize {
        self.items.iter().filter(|i| !i.reply.stays_open()).count()
    }

    /// How many it left open (not yet, or later).
    pub fn open(&self) -> usize {
        if self.whole { self.items.iter().filter(|i| i.reply.stays_open()).count() } else { 0 }
    }
}

fn reply_of(said: &str) -> Reply {
    match said.trim() {
        DONE => Reply::Done,
        YOURS => Reply::Yours,
        LATER => Reply::Later,
        YOUR_CALL => Reply::YourCall,
        DONT_KNOW => Reply::DontKnow,
        NOT_YET | "" => Reply::Empty,
        words => Reply::Text(words.to_owned()),
    }
}

/// `N) ` or `N. ` at a line's start: the number and the rest.
fn numbered<'a>(line: &'a str, mark: &str) -> Option<(usize, &'a str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let n = line[..digits].parse().ok()?;
    line[digits..].strip_prefix(mark).map(|rest| (n, rest))
}

/// The asks a message of answers carries, each with what came back, and the
/// person's words; also an older screen's (`1. ask` / `→ answer`, `2. ✓ ask`).
/// `None` when it is words of the person's own.
pub fn read(message: &str) -> Option<Read> {
    let mut lines = message.trim_start().lines().peekable();
    if lines.peek().is_some_and(|l| l.trim() == ANSWERS_HEAD) {
        lines.next();
        let mut out = Read { whole: true, ..Read::default() };
        let mut in_note = false;
        let mut rest = lines;
        while let Some(line) = rest.next() {
            if let Some(free) = line.strip_prefix(FROM) {
                let more: Vec<&str> = rest.by_ref().collect();
                out.free = std::iter::once(free).chain(more).collect::<Vec<_>>().join("\n").trim().to_owned();
                break;
            }
            if let Some((n, tail)) = numbered(line, ") ") {
                let (ask, said) = tail.split_once(" → ").unwrap_or((tail, ""));
                out.items.push(ReadItem { n, ask: ask.trim().to_owned(), reply: reply_of(said), note: String::new() });
                in_note = false;
                continue;
            }
            let (Some(more), Some(item)) = (line.strip_prefix(INDENT), out.items.last_mut()) else { continue };
            if let Some(note) = more.strip_prefix(NOTE) {
                item.note = note.trim_end().to_owned();
                in_note = true;
            } else if in_note {
                item.note = format!("{}\n{}", item.note, more.trim_end());
            } else if let Reply::Text(text) = &mut item.reply {
                text.push('\n');
                text.push_str(more.trim_end());
            }
        }
        return Some(out);
    }
    // An older screen's: `1. ask`, its answer under it after `→ `; `2. ✓ ask`.
    let mut out = Read::default();
    for line in message.lines() {
        if let Some((n, rest)) = numbered(line, ". ") {
            let (reply, ask) = match rest.strip_prefix("✓ ") {
                Some(ask) => (Reply::Done, ask),
                None => (Reply::Empty, rest),
            };
            out.items.push(ReadItem { n, ask: ask.trim().to_owned(), reply, note: String::new() });
        } else if let (Some(answer), Some(item)) = (line.strip_prefix("→ "), out.items.last_mut()) {
            item.reply = Reply::Text(answer.trim().to_owned());
        } else if let (Some(more), Some(item)) = (line.strip_prefix("  "), out.items.last_mut())
            && let Reply::Text(text) = &mut item.reply
        {
            text.push('\n');
            text.push_str(more.trim());
        }
    }
    (!out.items.is_empty()).then_some(out)
}

/// How many asks a message of answers answered. `None` when it isn't one
/// (words of the person's own), or answered none.
pub fn answered_count(message: &str) -> Option<usize> {
    read(message).map(|r| r.answered()).filter(|n| *n > 0)
}

/// The asks a message of answers leaves open — not yet answered, or put off
/// till later — whole, as `asks` has them. None for words of the person's own.
pub fn still_open(asks: &[String], message: &str) -> Vec<String> {
    let Some(read) = read(message).filter(|r| r.whole) else { return Vec::new() };
    read.items.iter().filter(|i| i.reply.stays_open()).filter_map(|i| asks.get(i.n.wrapping_sub(1)).cloned()).collect()
}

/// `asks`, and after them each of `open` that isn't among them already: what
/// a member asks again, with what was left open last time.
pub fn with_open(asks: &[String], open: &[String]) -> Vec<String> {
    let mut out = asks.to_vec();
    for ask in open {
        if !out.iter().any(|a| a.trim() == ask.trim()) {
            out.push(ask.clone());
        }
    }
    out
}

// ── what an ask wants ──

/// What the person gives back to an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Words: a name, a number, a choice, a date.
    Answer,
    /// A look and an OK — or words where it isn't OK.
    Confirm,
    /// A thing to do, ticked off when done.
    Do,
}

/// Words that ask for a look and an OK, in the languages equipe speaks.
const CONFIRMS: &[&str] = &[
    "괜찮", "확인해", "확인 부탁", "검토해", "맞는지", "봐 주세요", "봐주세요", "confirm", "looks good", "look over", "check that", "check the",
    "review the", "is it ok", "ok?", "approve", "確認", "确认", "確認して", "問題な", "没问题", "沒問題", "bestätig", "prüf", "passt", "vérifi",
    "confirme", "valide", "c'est bon", "revisa", "comprueba", "está bien", "de acuerdo",
];

/// Words that ask for words back.
const ANSWERS: &[&str] = &[
    "알려", "정해", "골라", "선택해", "적어", "답해", "말해", "보내 주", "주세요:", "무엇", "어떤", "언제", "누가", "어디", "얼마", "?", "？", "tell me",
    "let me know", "which", "what ", "when ", "who ", "where ", "how many", "how much", "decide", "choose", "pick", "provide", "share", "send me",
    "give me", "name the", "教え", "決め", "選ん", "どの", "いつ", "誰", "告诉", "告訴", "决定", "決定", "选择", "選擇", "提供", "哪", "什么", "什麼",
    "teile", "nenne", "sag mir", "gib mir", "entscheide", "wähle", "welche", "wann", "wer ", "dites", "indiquez", "choisissez", "donnez", "quel",
    "quand", "dime", "indica", "elige", "decide", "cuál", "cuándo", "quién",
];

/// What `ask` wants back: an OK when it asks for a look (and says what an OK
/// would be), words when it asks something, else a thing to do.
pub fn want_of(ask: &str) -> Want {
    // A link's `?` and words aren't the ask's: `…/new?plan=free` asks nothing.
    let lower: String = ask.to_lowercase().split_whitespace().filter(|w| !w.contains("://")).collect::<Vec<_>>().join(" ");
    // A Korean word counts at a word's start only: `설정해 주세요` is a thing
    // to do, not `정해 주세요`.
    let has = |words: &[&str]| {
        words.iter().any(|w| {
            let hangul = |c: char| ('\u{AC00}'..='\u{D7A3}').contains(&c);
            if !w.starts_with(hangul) {
                return lower.contains(w);
            }
            lower.match_indices(w).any(|(at, _)| !lower[..at].chars().next_back().is_some_and(hangul))
        })
    };
    if has(CONFIRMS) {
        Want::Confirm
    } else if has(ANSWERS) {
        Want::Answer
    } else {
        Want::Do
    }
}

// ── what the person picks ──

/// One of an ask's quiet choices beside its box or tick — one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    /// 했음: a thing to do, done.
    Done,
    /// 대신 해 줘: handed back, the member does it.
    Yours,
    /// 나중에: the member goes on without it; it stays open.
    Later,
    /// 네가 정해 줘: the member decides.
    YourCall,
    /// 모르겠어요: the member finds another way.
    DontKnow,
}

impl Pick {
    /// The reply it sends.
    pub fn reply(self) -> Reply {
        match self {
            Pick::Done => Reply::Done,
            Pick::Yours => Reply::Yours,
            Pick::Later => Reply::Later,
            Pick::YourCall => Reply::YourCall,
            Pick::DontKnow => Reply::DontKnow,
        }
    }
}

/// The choices an ask offers: a thing to do is done, handed back or put
/// off; a question (or a look) is answered in the box, or left to the member,
/// or not known.
pub fn picks_for(want: Want) -> &'static [Pick] {
    match want {
        Want::Do => &[Pick::Done, Pick::Yours, Pick::Later],
        Want::Answer | Want::Confirm => &[Pick::YourCall, Pick::DontKnow],
    }
}

/// The picks of a card after `pick` on ask `n`: one at a time — picking the
/// one already picked takes it back.
pub fn toggle(picks: &mut std::collections::HashMap<usize, Pick>, n: usize, pick: Pick) {
    if picks.get(&n) == Some(&pick) {
        picks.remove(&n);
    } else {
        picks.insert(n, pick);
    }
}

/// The replies a card holds, as the message builder takes them: a pick when
/// there is one, else words typed (a thing to do has no box).
pub fn replies(wants: &[Want], typed: &[String], picks: &std::collections::HashMap<usize, Pick>) -> Vec<Reply> {
    wants
        .iter()
        .enumerate()
        .map(|(n, want)| match (picks.get(&n), want) {
            (Some(pick), _) if picks_for(*want).contains(pick) => pick.reply(),
            (_, Want::Do) => Reply::Empty,
            _ => match typed.get(n).map(|t| t.trim()) {
                Some(t) if !t.is_empty() => Reply::Text(t.to_owned()),
                _ => Reply::Empty,
            },
        })
        .collect()
}

/// Everything a card holds, as it is sent.
pub fn answers(wants: &[Want], typed: &[String], picks: &std::collections::HashMap<usize, Pick>, notes: &[String], free: &str) -> Answers {
    Answers { replies: replies(wants, typed, picks), notes: notes.to_vec(), free: free.trim().to_owned() }
}

/// Whether the card can be sent: anything set — an answer, a pick, a note,
/// or words to the member alone.
pub fn can_send(answers: &Answers) -> bool {
    !answers.is_empty()
}

impl crate::Ticket {
    /// What it asks of the person while it waits for them, item by item: the
    /// list it carries; from an older daemon, its note's checklist; from
    /// before that, the one line the asks were joined into.
    pub fn asks(&self) -> Vec<String> {
        if !self.asks.is_empty() {
            return self.asks.clone();
        }
        let Some(note) = self.note.as_deref().filter(|_| self.status == crate::TicketStatus::Waiting) else { return Vec::new() };
        let listed = waiting_needs(note);
        if !listed.is_empty() {
            return listed;
        }
        let line = note.trim().strip_prefix(WAITING_HEAD).unwrap_or(note).trim();
        if line.is_empty() { Vec::new() } else { split_joined(line) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_to_make_is_a_thing_to_do_not_a_question() {
        assert_eq!(want_of("대상은 '내부(Internal)'로 설정해 주세요."), Want::Do);
        assert_eq!(want_of("어느 쪽으로 할지 정해 주세요"), Want::Answer);
        assert_eq!(want_of("이름을 골라 주세요"), Want::Answer);
        assert_eq!(want_of("https://github.com/account/organizations/new?plan=free 에 들어가 조직을 만들어 주세요"), Want::Do, "a link's ? asks nothing");
    }

    /// The line the person saw, its asks joined with ", ".
    const JOINED: &str = "심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만 보고 공개되지 않습니다., 회사 iOS 엔지니어 리뷰를 언제, 누가 할지 정해 주세요. equipe-app 규칙상 스토어 출시 전에 꼭 받아야 합니다., 스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ (en·ko 5장씩, 실제 앱 화면) 와 listing.json. ko 스크린샷 속 티켓 제목은 데모 데이터라 영어입니다. 그대로 둬도 되면 '괜찮음'이라고만 답해 주세요.";

    #[test]
    fn the_joined_line_splits_back_into_its_three_asks() {
        let asks = split_joined(JOINED);
        assert_eq!(asks.len(), 3, "{asks:#?}");
        assert_eq!(asks[0], "심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만 보고 공개되지 않습니다.");
        assert!(asks[1].starts_with("회사 iOS 엔지니어 리뷰를 언제, 누가 할지") && asks[1].ends_with("받아야 합니다."));
        assert!(asks[2].starts_with("스크린샷과 문구를") && asks[2].ends_with("답해 주세요."));
        // One ask, commas and all, stays one.
        assert_eq!(split_joined("이름, 전화번호, 이메일을 알려 주세요"), ["이름, 전화번호, 이메일을 알려 주세요"]);
        assert_eq!(split_joined("Create the key., Add it to Vercel?, Restart"), ["Create the key.", "Add it to Vercel?", "Restart"]);
        assert!(split_joined("  ").is_empty());
    }

    #[test]
    fn a_note_keeps_each_ask_whole_line_breaks_and_all() {
        let asks = ["GitHub token".to_owned(), "Check the shots:\n- en\n- ko".to_owned(), " ".to_owned()];
        let note = waiting_note(&asks[..2], Some("for the deploy"));
        assert_eq!(note, "Waiting on you:\n- [ ] GitHub token\n- [ ] Check the shots:\n  - en\n  - ko\n(for the deploy)");
        assert_eq!(waiting_needs(&note), ["GitHub token", "Check the shots:\n- en\n- ko"]);
        assert_eq!(waiting_why(&note).as_deref(), Some("for the deploy"));
        assert_eq!(waiting_why("Resumed: (x)"), None);
        assert_eq!(waiting_needs("Waiting on you:\n- [ ] a\n- [ ] b"), ["a", "b"]);
    }

    #[test]
    fn a_ticket_reads_its_list_else_its_note_else_the_joined_line() {
        let mut t: crate::Ticket = serde_json::from_value(serde_json::json!({
            "id": 117, "title": "앱스토어 제출", "body": "", "status": "waiting", "owner": "ios", "assignee": "ios",
            "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": format!("{WAITING_HEAD} {JOINED}"),
        }))
        .unwrap();
        assert!(t.asks.is_empty(), "an older daemon's ticket has no list");
        assert_eq!(t.asks().len(), 3);
        t.note = Some(waiting_note(&["a".into(), "b".into()], None));
        assert_eq!(t.asks(), ["a", "b"]);
        t.asks = vec!["one".into()];
        assert_eq!(t.asks(), ["one"], "the list it carries comes first");
        t.asks.clear();
        t.status = crate::TicketStatus::Todo;
        assert!(t.asks().is_empty(), "not waiting: nothing asked");
    }

    /// T-117 as the person saw it: one question, one thing to do.
    fn t117() -> Vec<String> {
        vec![
            "App Review 연락처를 알려 주세요: 전화번호(국가번호 포함)와 이메일.".into(),
            "App Store Connect에서 EU DSA 상태를 trader로 바꿔 주세요.".into(),
        ]
    }

    fn answers(replies: &[Reply], notes: &[&str], free: &str) -> Answers {
        Answers { replies: replies.to_vec(), notes: notes.iter().map(|n| (*n).to_owned()).collect(), free: free.into() }
    }

    #[test]
    fn answers_go_back_numbered_with_what_came_back_and_the_persons_words_last() {
        let asks = t117();
        let message = answer_message(&asks, &answers(&[Reply::Text("+82 10 1234 5678\nreview@equipe.im".into()), Reply::Yours], &["", "2FA 코드는 제 폰으로 와요"], "급하지 않아요"));
        assert_eq!(
            message,
            "Answers to what you asked for:\n\
             1) App Review 연락처를 알려 주세요: 전화번호(국가번호 포함)와 이메일. → +82 10 1234 5678\n   review@equipe.im\n\
             2) App Store Connect에서 EU DSA 상태를 trader로 바꿔 주세요. → do it yourself — the person handed this back to you\n   Note: 2FA 코드는 제 폰으로 와요\n\
             From the person: 급하지 않아요"
        );
        // Every reply says what it means.
        let one = |reply: Reply| answer_message(&asks[1..], &answers(&[reply], &[], ""));
        assert!(one(Reply::Done).ends_with("→ done ✓"));
        assert!(one(Reply::Later).ends_with("→ later — go on without it; it stays open on the ticket"));
        assert!(one(Reply::YourCall).ends_with("→ your call — decide it yourself and say what you chose"));
        assert!(one(Reply::DontKnow).ends_with("→ the person doesn't know — find another way, or ask it differently"));
        assert!(one(Reply::Empty).ends_with("→ — (not yet: still open on the ticket)"));
        assert!(one(Reply::Text("  ".into())).ends_with("(not yet: still open on the ticket)"), "blank words are no answer");
        // A long ask is quoted by its start; an arrow in it can't be taken for the answer's.
        let long = vec![format!("{} → 그리고 끝", "가".repeat(100))];
        let quoted = answer_message(&long, &answers(&[Reply::Done], &[], ""));
        assert!(quoted.contains(&format!("1) {}… → done ✓", "가".repeat(80))), "{quoted}");
        assert_eq!(read(&answer_message(&["a → b".to_owned()], &answers(&[Reply::Done], &[], ""))).unwrap().items[0].reply, Reply::Done);
    }

    #[test]
    fn a_partial_answer_lists_what_is_left_and_keeps_it_open() {
        let asks = t117();
        let message = answer_message(&asks, &answers(&[Reply::Text("+82 10 1234 5678".into())], &[], ""));
        assert!(message.ends_with("2) App Store Connect에서 EU DSA 상태를 trader로 바꿔 주세요. → — (not yet: still open on the ticket)"), "{message}");
        assert_eq!(still_open(&asks, &message), [asks[1].clone()]);
        let read = read(&message).unwrap();
        assert_eq!((read.answered(), read.open()), (1, 1));
        assert_eq!(answered_count(&message), Some(1));
        // Later stays open too; handed back, decided or not known doesn't.
        for (reply, open) in [(Reply::Later, true), (Reply::Yours, false), (Reply::Done, false), (Reply::YourCall, false), (Reply::DontKnow, false)] {
            let m = answer_message(&asks, &answers(&[Reply::Text("x".into()), reply.clone()], &[], ""));
            assert_eq!(still_open(&asks, &m).len(), usize::from(open), "{reply:?}");
        }
        // A member asks again with what was left, once.
        assert_eq!(with_open(&["새 질문".into(), asks[1].clone()], &[asks[1].clone()]), ["새 질문".to_owned(), asks[1].clone()]);
        assert_eq!(with_open(&["새 질문".into()], &[asks[1].clone()]), ["새 질문".to_owned(), asks[1].clone()]);
    }

    #[test]
    fn the_persons_words_alone_still_go_and_leave_the_asks_open() {
        let asks = t117();
        let only = answers(&[], &[], "이건 너가 직접 해");
        assert!(!only.is_empty() && answers(&[Reply::Text(" ".into())], &[" "], "\n").is_empty());
        let message = answer_message(&asks, &only);
        assert!(message.starts_with(ANSWERS_HEAD) && message.ends_with("\nFrom the person: 이건 너가 직접 해"), "{message}");
        let back = read(&message).unwrap();
        assert_eq!((back.answered(), back.open(), back.free.as_str()), (0, 2, "이건 너가 직접 해"));
        assert_eq!(answered_count(&message), None, "nothing answered");
        // A note alone is something said.
        assert!(!answers(&[], &["", "메모"], "").is_empty());
        // Nothing asked: their words, as they are.
        assert_eq!(answer_message(&[], &only), "이건 너가 직접 해");
        assert_eq!(read("both are in .env.local"), None);
    }

    #[test]
    fn every_combination_reads_back_as_it_was_sent() {
        let asks: Vec<String> = vec!["q1?".into(), "do 2".into(), "q3?".into()];
        let choices = [
            Reply::Empty,
            Reply::Text("+82 10\nsecond line".into()),
            Reply::Done,
            Reply::Yours,
            Reply::Later,
            Reply::YourCall,
            Reply::DontKnow,
        ];
        let notes = ["", "메모 하나", "두 줄\n메모"];
        for a in &choices {
            for b in &choices {
                for note in notes {
                    for free in ["", "끝에 한마디\n그리고 둘째 줄"] {
                        let sent = answers(&[a.clone(), b.clone(), Reply::Empty], &[note, "", note], free);
                        let message = answer_message(&asks, &sent);
                        let back = read(&message).unwrap_or_else(|| panic!("{message}"));
                        assert!(back.whole);
                        assert_eq!(back.items.iter().map(|i| i.reply.clone()).collect::<Vec<_>>(), [a.clone(), b.clone(), Reply::Empty], "{message}");
                        assert_eq!(back.items.iter().map(|i| i.note.as_str()).collect::<Vec<_>>(), [note, "", note], "{message}");
                        assert_eq!(back.items.iter().map(|i| i.ask.as_str()).collect::<Vec<_>>(), ["q1?", "do 2", "q3?"]);
                        assert_eq!(back.free, free);
                    }
                }
            }
        }
    }

    #[test]
    fn one_pick_at_a_time_and_it_wins_over_the_box() {
        let wants = [Want::Answer, Want::Do, Want::Confirm];
        let mut picks = std::collections::HashMap::new();
        toggle(&mut picks, 1, Pick::Done);
        toggle(&mut picks, 1, Pick::Yours);
        assert_eq!(picks.get(&1), Some(&Pick::Yours), "one at a time");
        toggle(&mut picks, 1, Pick::Yours);
        assert_eq!(picks.get(&1), None, "picked again: taken back");
        toggle(&mut picks, 0, Pick::YourCall);
        toggle(&mut picks, 1, Pick::Later);
        let typed = vec!["홍길동".to_owned(), String::new(), " ".to_owned()];
        assert_eq!(replies(&wants, &typed, &picks), [Reply::YourCall, Reply::Later, Reply::Empty], "a pick wins over words typed");
        picks.remove(&0);
        assert_eq!(replies(&wants, &typed, &picks)[0], Reply::Text("홍길동".into()));
        // A pick an ask doesn't offer counts for nothing.
        picks.insert(2, Pick::Done);
        assert_eq!(replies(&wants, &typed, &picks)[2], Reply::Empty);
        assert_eq!(picks_for(Want::Do), [Pick::Done, Pick::Yours, Pick::Later]);
        assert_eq!(picks_for(Want::Confirm), [Pick::YourCall, Pick::DontKnow]);
        // Anything set can go; nothing can't.
        let none = std::collections::HashMap::new();
        assert!(!can_send(&super::answers(&wants, &[], &none, &[], " ")));
        assert!(can_send(&super::answers(&wants, &[], &none, &[], "이건 너가 직접 해")));
        assert!(can_send(&super::answers(&wants, &[], &none, &[String::new(), "메모".into()], "")));
        assert!(can_send(&super::answers(&wants, &typed, &none, &[], "")));
    }

    #[test]
    fn an_older_screens_answers_still_read() {
        let old = "1. 심사 연락처를 알려 주세요\n→ 홍길동\n  +82 10 1234 5678\n\n2. ✓ Stripe 대시보드에서 테스트 키 만들기\n\n3. 스크린샷과 문구를 확인해 주세요…\n→ 괜찮음";
        let read = read(old).unwrap();
        assert!(!read.whole);
        assert_eq!(
            read.items.iter().map(|i| (i.n, i.reply.clone())).collect::<Vec<_>>(),
            [(1, Reply::Text("홍길동\n+82 10 1234 5678".into())), (2, Reply::Done), (3, Reply::Text("괜찮음".into()))]
        );
        assert_eq!(answered_count(old), Some(3));
        assert!(still_open(&t117(), old).is_empty(), "it never listed what it left");
        assert_eq!(answered_count("both are in .env.local"), None);
        assert!(Reply::Text(" ".into()).is_empty() && !Reply::Done.is_empty());
    }
}
