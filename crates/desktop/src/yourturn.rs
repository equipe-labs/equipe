//! A ticket put down for the person (`block_on_person`), as the card they
//! answer it on — in the conversation and in the Inbox's peek alike: what
//! each ask wants (words, an OK, or a thing done), the files it names, the one
//! message the answers go back in, and where the card stands — waiting,
//! sending, answered here or on another screen. No UI types here, so the rules
//! are tested without a window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use equipe_core::protocol::asks;
pub use equipe_core::protocol::asks::{Pick, Want, answers, can_send, picks_for, toggle, want_of};
use equipe_core::protocol::{Ticket, TicketId, TicketStatus};

// ── files it names ──

/// A file or folder an ask names: as written, and where it is on this Mac
/// when it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub written: String,
    pub at: Option<PathBuf>,
}

/// Kinds of file a name alone (no folder) is taken for.
const KINDS: &[&str] = &[
    "json", "md", "txt", "csv", "pdf", "png", "jpg", "jpeg", "gif", "webp", "svg", "mp4", "mov", "zip", "yaml", "yml", "toml", "html", "css", "js", "ts",
    "tsx", "jsx", "rs", "py", "swift", "kt", "sh", "env", "plist", "xlsx", "docx", "pptx", "key", "pem", "ipa", "apk", "aab", "log", "sql",
];

/// Marks around a word that aren't part of a path.
fn trim_marks(word: &str) -> &str {
    word.trim_start_matches(['(', '[', '"', '\'', '“', '‘', '「', '『', '<'])
        .trim_end_matches([')', ']', '"', '\'', '”', '’', '」', '』', '>', ',', ';', ':', '!', '?', '、', '，', '。', '：'])
}

/// A sentence's end after a file's name (`listing.json.`), and Korean's
/// particles on it (`listing.json을`), off.
fn trim_tail(word: &str) -> &str {
    let mut w = trim_marks(word);
    while let Some(c) = w.chars().last() {
        let hangul = ('\u{AC00}'..='\u{D7A3}').contains(&c);
        let after_name = w[..w.len() - c.len_utf8()].rsplit_once('.').is_some_and(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()));
        if (hangul && after_name) || (c == '.' && !w.ends_with("/.")) {
            w = &w[..w.len() - c.len_utf8()];
        } else {
            break;
        }
    }
    w
}

/// Whether a word reads as a path: a folder in it, or a known kind of file.
fn pathish(word: &str) -> bool {
    if word.len() < 3 || word.contains("://") || word.starts_with("T-") && !word.contains('/') {
        return false;
    }
    let parts = word.split('/').filter(|p| !p.is_empty()).count();
    if word.contains('/') && (parts >= 2 || word.ends_with('/') || word.starts_with('/') || word.starts_with("~/")) {
        // `3/4` is a number.
        return word.chars().any(char::is_alphabetic);
    }
    // `.env.local`: a dotfile.
    let dotfile = word.starts_with('.') && word[1..].chars().next().is_some_and(char::is_alphabetic);
    dotfile || word.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && KINDS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Where `written` is, under the first of `roots` that has it.
fn locate(written: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(written);
    if let Some(rest) = written.strip_prefix("~/") {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        return Some(home.join(rest)).filter(|p| p.exists());
    }
    if path.is_absolute() {
        return path.exists().then(|| path.to_owned());
    }
    roots.iter().map(|r| r.join(written.trim_start_matches("./"))).find(|p| p.exists())
}

/// The files and folders `text` names: each code span that reads as a path,
/// and each such word — a folder whose name has spaces taken whole when it
/// is on this Mac under one of `roots`, or when a later word goes on with a
/// `/` (`outputs/T-117 앱스토어 제출/shots/`).
pub fn named(text: &str, roots: &[PathBuf]) -> Vec<Named> {
    let mut out: Vec<Named> = Vec::new();
    let mut push = |written: &str, at: Option<PathBuf>| {
        let written = written.trim().to_owned();
        if !written.is_empty() && !out.iter().any(|n| n.written == written) {
            out.push(Named { at, written });
        }
    };
    // Code spans first: the member marked them.
    let mut plain = String::new();
    for (n, part) in text.split('`').enumerate() {
        if n % 2 == 1 {
            if pathish(part.trim()) || locate(part.trim(), roots).is_some() {
                push(part, locate(part.trim(), roots));
            }
            plain.push(' ');
        } else {
            plain.push_str(part);
        }
    }
    let words: Vec<&str> = plain.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let first = trim_marks(words[i]);
        if !first.contains('/') && !pathish(trim_tail(words[i])) {
            i += 1;
            continue;
        }
        // The longest run from here that is on this Mac, else that goes on with a `/`.
        let mut best: Option<(usize, String, Option<PathBuf>)> = None;
        for end in (i..words.len().min(i + 6)).rev() {
            let joined = words[i..=end].join(" ");
            let candidate = trim_tail(&joined);
            if let Some(at) = locate(candidate, roots) {
                best = Some((end, candidate.to_owned(), Some(at)));
                break;
            }
        }
        if best.is_none() && first.contains('/') {
            // Words without marks between, the last of them with a `/`.
            let mut end = i;
            for (k, w) in words.iter().enumerate().skip(i + 1).take(3) {
                if w.chars().any(|c| matches!(c, '(' | ')' | ',' | ':' | '·' | '，' | '：')) {
                    break;
                }
                if w.contains('/') {
                    end = k;
                }
            }
            let joined = words[i..=end].join(" ");
            let candidate = trim_tail(&joined);
            if pathish(candidate) {
                best = Some((end, candidate.to_owned(), None));
            }
        }
        match best {
            Some((end, written, at)) => {
                push(&written, at);
                i = end + 1;
            }
            None => {
                let word = trim_tail(words[i]);
                if pathish(word) {
                    push(word, locate(word, roots));
                }
                i += 1;
            }
        }
    }
    out
}

/// `text` with each path it names (`named`) in a code span, so it reads as
/// a path — not prose, nor a ticket (`outputs/T-117 …/`) — unless it is in
/// one already.
pub fn mark_paths(text: &str, named: &[Named]) -> String {
    let mut out = text.to_owned();
    for n in named {
        let Some(at) = out.find(n.written.as_str()) else { continue };
        let inside = out[..at].matches('`').count() % 2 == 1;
        if !inside {
            out.replace_range(at..at + n.written.len(), &format!("`{}`", n.written));
        }
    }
    out
}

// ── where the card stands ──

/// What the person answered a ticket's asks with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    /// The message that went to the member (empty: done, no words).
    pub text: String,
    /// When, in unix seconds; `None` when only the ticket's moving told us.
    pub at: Option<u64>,
    /// Sent from this window's card.
    pub here: bool,
    /// The daemon took it: the ticket is no longer waiting.
    pub settled: bool,
}

/// Where a card stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the person.
    Pending,
    /// Sent from here; the daemon hasn't said yet.
    Sending,
    /// Answered — `here`, or on another screen (the board, a phone, the terminal).
    Answered { here: bool },
    /// It stopped waiting without an answer (cancelled).
    Closed,
}

pub fn phase(ticket: Option<&Ticket>, answered: Option<&Answered>) -> Phase {
    match (ticket.map(|t| t.status), answered) {
        (Some(TicketStatus::Waiting), Some(a)) if a.here && !a.settled => Phase::Sending,
        (Some(TicketStatus::Waiting), _) => Phase::Pending,
        (_, Some(a)) => Phase::Answered { here: a.here },
        (Some(TicketStatus::Cancelled), None) => Phase::Closed,
        _ => Phase::Answered { here: false },
    }
}

/// What the daemon writes on a ticket the person resumed, before their words.
const RESUMED: &str = "Resumed: ";
/// …and in their place when there were none.
const RESUMED_EMPTY: &str = "Resumed: the person did what you were waiting for.";

/// Sent from here: the card says so until the daemon moves the ticket.
pub fn sent(answers: &mut HashMap<TicketId, Answered>, id: TicketId, text: String, now: u64) {
    answers.insert(id, Answered { text, at: Some(now), here: true, settled: false });
}

/// A ticket moved, from `was` to how it is now: a wait answered here is
/// settled; one answered on another screen is taken from what the ticket
/// says now; a new wait starts with no answer. Whether anything changed.
pub fn on_ticket(answers: &mut HashMap<TicketId, Answered>, was: Option<TicketStatus>, ticket: &Ticket, now: u64) -> bool {
    let waiting = ticket.status == TicketStatus::Waiting;
    match (was, waiting) {
        (Some(TicketStatus::Waiting), false) => {
            if let Some(mine) = answers.get_mut(&ticket.id) {
                mine.settled = true;
                return true;
            }
            if ticket.status == TicketStatus::Cancelled {
                return false;
            }
            let said = ticket.note.as_deref().unwrap_or_default().trim();
            let text = if said == RESUMED_EMPTY { "" } else { said.strip_prefix(RESUMED).unwrap_or("") };
            answers.insert(ticket.id, Answered { text: text.trim().to_owned(), at: Some(now), here: false, settled: true });
            true
        }
        // Put down again: a new round of asks.
        (Some(w), true) if w != TicketStatus::Waiting => answers.remove(&ticket.id).is_some(),
        _ => false,
    }
}

/// How many asks an answer answered, when it says.
pub fn answered_count(a: &Answered) -> Option<usize> {
    asks::answered_count(&a.text)
}

/// What it left open, when it says (not yet, or later).
pub fn open_count(a: &Answered) -> usize {
    asks::read(&a.text).map_or(0, |r| r.open())
}

/// The conversation's cards brought up to date: each ticket's latest is
/// live (an earlier one, asked again since, is past), and carries how its
/// card looks now (`looks`, by ticket), so a change draws it again.
pub fn mark(blocks: &mut [crate::convo::Block], looks: &HashMap<TicketId, u64>) {
    let mut seen = std::collections::HashSet::new();
    for block in blocks.iter_mut().rev() {
        if let crate::convo::Block::YourTurn { ticket, live, look, .. } = block {
            *live = seen.insert(*ticket);
            *look = if *live { looks.get(ticket).copied().unwrap_or(0) } else { 0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASKS: [&str; 3] = [
        "심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만 보고 공개되지 않습니다.",
        "회사 iOS 엔지니어 리뷰를 언제, 누가 할지 정해 주세요. equipe-app 규칙상 스토어 출시 전에 꼭 받아야 합니다.",
        "스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ (en·ko 5장씩, 실제 앱 화면) 와 listing.json. ko 스크린샷 속 티켓 제목은 데모 데이터라 영어입니다. 그대로 둬도 되면 '괜찮음'이라고만 답해 주세요.",
    ];

    fn ticket(id: TicketId, status: &str, note: Option<&str>) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "앱스토어 제출", "body": "", "status": status, "owner": "ios", "assignee": "ios", "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": note})).unwrap()
    }

    #[test]
    fn each_ask_wants_words_an_ok_or_a_thing_done() {
        assert_eq!(ASKS.map(want_of), [Want::Answer, Want::Answer, Want::Confirm]);
        assert_eq!(want_of("Stripe 대시보드에서 테스트 키 만들기"), Want::Do);
        assert_eq!(want_of("Add STRIPE_KEY to the Vercel project"), Want::Do);
        assert_eq!(want_of("Which plan should we take?"), Want::Answer);
        assert_eq!(want_of("Check the copy in listing.json and confirm"), Want::Confirm);
        assert_eq!(want_of("運用 DB のパスワードを教えてください"), Want::Answer);
    }

    #[test]
    fn a_folder_with_spaces_and_a_bare_file_are_named() {
        let found = named(ASKS[2], &[]);
        assert_eq!(found.iter().map(|n| n.written.as_str()).collect::<Vec<_>>(), ["outputs/T-117 앱스토어 제출/screenshots-6.9/", "listing.json"]);
        assert!(found.iter().all(|n| n.at.is_none()), "nothing of it on this Mac");
        assert!(named(ASKS[0], &[]).is_empty(), "no path in it");
        assert_eq!(named("Put it in `.env.local` and `src/app.ts`.", &[]).iter().map(|n| n.written.as_str()).collect::<Vec<_>>(), [".env.local", "src/app.ts"]);
        assert_eq!(named("listing.json을 봐 주세요", &[])[0].written, "listing.json");
        assert!(named("T-117 and 3/4 of it, e.g. this", &[]).is_empty());
    }

    #[test]
    fn the_paths_an_ask_names_read_as_paths() {
        let found = named(ASKS[2], &[]);
        let marked = mark_paths(ASKS[2], &found);
        assert!(marked.contains("`outputs/T-117 앱스토어 제출/screenshots-6.9/` (en·ko") && marked.contains("와 `listing.json`. ko"), "{marked}");
        let spanned = "Put it in `src/app.ts` now";
        assert_eq!(mark_paths(spanned, &named(spanned, &[])), spanned, "in a code span already");
    }

    #[test]
    fn a_folder_on_this_mac_is_found_whole_and_opens() {
        let root = std::env::temp_dir().join(format!("equipe-turn-{}", std::process::id()));
        let shots = root.join("outputs/T-117 앱스토어 제출/screenshots-6.9");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(root.join("listing.json"), "{}").unwrap();
        let found = named("확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9 (en·ko) 와 listing.json.", std::slice::from_ref(&root));
        assert_eq!(found[0], Named { written: "outputs/T-117 앱스토어 제출/screenshots-6.9".into(), at: Some(shots) });
        assert_eq!(found[1].at, Some(root.join("listing.json")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_card_goes_from_waiting_to_answered_here_or_elsewhere() {
        let mut answers = HashMap::new();
        let waiting = ticket(117, "waiting", Some("Waiting on you:\n- [ ] a"));
        assert_eq!(phase(Some(&waiting), answers.get(&117)), Phase::Pending);

        // Answered here: sending until the daemon moves the ticket, then answered.
        sent(&mut answers, 117, "1. a\n→ yes".into(), 100);
        assert_eq!(phase(Some(&waiting), answers.get(&117)), Phase::Sending);
        let resumed = ticket(117, "todo", Some("Resumed: 1. a\n→ yes"));
        assert!(on_ticket(&mut answers, Some(TicketStatus::Waiting), &resumed, 101));
        assert_eq!(phase(Some(&resumed), answers.get(&117)), Phase::Answered { here: true });
        assert_eq!(answers[&117].at, Some(100), "when it was sent");
        assert_eq!(answered_count(&answers[&117]), Some(1));

        // Answered on another screen: what the ticket says now is what was answered.
        let other = ticket(55, "waiting", None);
        assert!(!on_ticket(&mut answers, None, &other, 0), "the snapshot is no news");
        let moved = ticket(55, "in_progress", Some("Resumed: 1. 키\n→ sk_test_1\n\n2. ✓ Vercel"));
        assert!(on_ticket(&mut answers, Some(TicketStatus::Waiting), &moved, 200));
        assert_eq!(phase(Some(&moved), answers.get(&55)), Phase::Answered { here: false });
        assert_eq!(answers[&55].text, "1. 키\n→ sk_test_1\n\n2. ✓ Vercel");
        assert_eq!(answered_count(&answers[&55]), Some(2));
        // The board's "done" with no words.
        let plain = ticket(56, "todo", Some(RESUMED_EMPTY));
        on_ticket(&mut answers, Some(TicketStatus::Waiting), &plain, 300);
        assert_eq!(answers[&56].text, "");

        // Put down again: a new round, nothing answered yet.
        let again = ticket(117, "waiting", Some("Waiting on you:\n- [ ] b"));
        assert!(on_ticket(&mut answers, Some(TicketStatus::InProgress), &again, 400));
        assert_eq!(phase(Some(&again), answers.get(&117)), Phase::Pending);

        // Cancelled while it waited: closed, not answered.
        let gone = ticket(73, "cancelled", None);
        assert!(!on_ticket(&mut answers, Some(TicketStatus::Waiting), &gone, 500));
        assert_eq!(phase(Some(&gone), answers.get(&73)), Phase::Closed);
    }

    #[test]
    fn a_tickets_latest_card_is_live_and_redrawn_when_it_changes() {
        use crate::convo::Block;
        let card = |ticket| Block::YourTurn { ticket, news: String::new(), live: false, look: 0 };
        let mut blocks = vec![card(117), Block::Note("n".into()), card(55), card(117)];
        let looks: HashMap<TicketId, u64> = [(117, 7), (55, 9)].into_iter().collect();
        mark(&mut blocks, &looks);
        let marks: Vec<(bool, u64)> = blocks.iter().filter_map(|b| if let Block::YourTurn { live, look, .. } = b { Some((*live, *look)) } else { None }).collect();
        assert_eq!(marks, [(false, 0), (true, 9), (true, 7)]);
    }

    #[test]
    fn the_card_sends_what_was_set_and_says_what_it_left_open() {
        let asks: Vec<String> = vec!["App Review 연락처를 알려 주세요.".into(), "EU DSA 상태를 trader로 바꿔 주세요.".into()];
        let wants: Vec<Want> = asks.iter().map(|a| want_of(a)).collect();
        assert_eq!(wants, [Want::Answer, Want::Do]);
        let typed = vec!["+82 10 1234 5678".to_owned(), String::new()];
        let none = HashMap::new();
        assert!(!can_send(&answers(&wants, &[], &none, &[], "")), "nothing set yet");
        // Partly answered: what is left goes as not yet, and the card says it stayed open.
        let partial = answers(&wants, &typed, &none, &[], "");
        assert!(can_send(&partial));
        let a = Answered { text: asks::answer_message(&asks, &partial), at: Some(1), here: true, settled: true };
        assert_eq!((answered_count(&a), open_count(&a)), (Some(1), 1));
        // Handed back with a note: nothing left open.
        let picks: HashMap<usize, Pick> = [(1, Pick::Yours)].into_iter().collect();
        let whole = answers(&wants, &typed, &picks, &[String::new(), "2FA는 제 폰으로 와요".into()], "");
        let a = Answered { text: asks::answer_message(&asks, &whole), ..a };
        assert_eq!((answered_count(&a), open_count(&a)), (Some(2), 0));
        // An older screen's answer leaves nothing it can tell open.
        let old = Answered { text: "1. 키\n→ sk_test_1\n\n2. ✓ Vercel".into(), ..a };
        assert_eq!((answered_count(&old), open_count(&old)), (Some(2), 0));
    }
}
