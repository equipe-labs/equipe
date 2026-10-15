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

/// What the person writes back to one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Nothing yet.
    Empty,
    /// Their words.
    Text(String),
    /// Done (a thing to do, ticked off).
    Done,
}

impl Reply {
    pub fn is_empty(&self) -> bool {
        match self {
            Reply::Empty => true,
            Reply::Text(t) => t.trim().is_empty(),
            Reply::Done => false,
        }
    }
}

/// Longer than this, an ask is quoted by its start in the answer.
const QUOTED: usize = 80;

/// The one message the person's answers go to the member in: each ask they
/// answered, numbered as it was asked and quoted by its start, the answer
/// under it (`→ …`); one they ticked off, `✓`. Nothing for the ones left.
pub fn answer_message(asks: &[String], replies: &[Reply]) -> String {
    let quote = |ask: &str| {
        let first = ask.lines().next().unwrap_or_default().trim();
        if first.chars().count() > QUOTED || ask.lines().count() > 1 {
            format!("{}…", first.chars().take(QUOTED).collect::<String>().trim_end())
        } else {
            first.to_owned()
        }
    };
    let mut parts = Vec::new();
    for (n, ask) in asks.iter().enumerate() {
        match replies.get(n) {
            Some(Reply::Text(text)) if !text.trim().is_empty() => {
                let answer = text.trim().lines().map(str::trim_end).collect::<Vec<_>>().join("\n  ");
                parts.push(format!("{}. {}\n→ {answer}", n + 1, quote(ask)));
            }
            Some(Reply::Done) => parts.push(format!("{}. ✓ {}", n + 1, quote(ask))),
            _ => {}
        }
    }
    parts.join("\n\n")
}

/// How many asks a message of [`answer_message`]'s answers: its numbered
/// lines. `None` when it isn't one (words of the person's own).
pub fn answered_count(message: &str) -> Option<usize> {
    let n = message
        .lines()
        .filter(|l| {
            let digits = l.chars().take_while(char::is_ascii_digit).count();
            digits > 0 && l[digits..].starts_with(". ")
        })
        .count();
    (n > 0).then_some(n)
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

    #[test]
    fn answers_go_back_numbered_under_what_they_answer() {
        let asks: Vec<String> = vec![
            "심사 연락처를 알려 주세요: 이름, 전화번호, 이메일.".into(),
            "Stripe 대시보드에서 테스트 키 만들기".into(),
            "스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ 와 listing.json. 그대로 둬도 되면 '괜찮음'이라고만 답해 주세요.".into(),
            "언제 할지 정해 주세요".into(),
        ];
        let replies = [Reply::Text("홍길동\n+82 10 1234 5678".into()), Reply::Done, Reply::Text("괜찮음".into()), Reply::Text("  ".into())];
        let message = answer_message(&asks, &replies);
        assert_eq!(
            message,
            "1. 심사 연락처를 알려 주세요: 이름, 전화번호, 이메일.\n→ 홍길동\n  +82 10 1234 5678\n\n2. ✓ Stripe 대시보드에서 테스트 키 만들기\n\n3. 스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ 와 listing.json. 그대로 둬도…\n→ 괜찮음"
        );
        assert_eq!(answered_count(&message), Some(3));
        assert_eq!(answered_count("both are in .env.local"), None);
        assert_eq!(answer_message(&asks, &[]), "");
        assert!(Reply::Text(" ".into()).is_empty() && !Reply::Done.is_empty());
    }
}
