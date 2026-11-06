//! Deciding in the person's own words, on this screen: what waits on them
//! in words (`equipe_protocol::by_words::Live`, the daemon's), whether "all
//! by words" is on for the squad (the daemon keeps it; only the Mac's
//! screens change it), what the composer sends for their message — `say`,
//! with how it came in, to a daemon that reads words; `submit` to one from
//! before — and the words for the card over the composer and the log of
//! where words went wrong. No UI types here, so the rules are tested
//! without a window. The reading and the deciding are the daemon's
//! (`equipe_core::by_words`).
//!
//! A paste is known only as ⌘V into the box (`views/composer.rs`): text
//! dragged in, a Services menu, an autocompletion or an input method's
//! suggestion reach the box as typed, and count as the person's own words.

use equipe_core::protocol::by_words::{Entry, Flag, Live, Outcome, Reply, Stage, Tier, Via};
use equipe_core::protocol::Command;
use equipe_core::t;

use crate::sheet::{Ask, Button, Row, Sheet, Then};

/// What waits in words for the squad on screen, and the log once asked for.
#[derive(Debug, Clone, Default)]
pub struct Words {
    /// The daemon reads the person's words (its snapshot says so).
    pub read: bool,
    /// "All by words" is on for the squad, as the daemon keeps it.
    pub all: bool,
    live: Vec<Live>,
    /// The log, as the daemon sent it, waiting to be shown.
    log: Option<Vec<Entry>>,
}

impl Words {
    /// The snapshot's: what waits replaces everything — `None` from a daemon
    /// that reads no words.
    pub fn load(&mut self, words: Option<Vec<Live>>, all: bool) {
        self.read = words.is_some();
        self.live = words.unwrap_or_default();
        self.all = all;
    }

    /// New, or a step on.
    pub fn live(&mut self, live: Live) {
        match self.live.iter_mut().find(|l| l.id == live.id) {
            Some(was) => *was = live,
            None => self.live.push(live),
        }
    }

    pub fn close(&mut self, id: u64) {
        self.live.retain(|l| l.id != id);
    }

    pub fn any(&self) -> bool {
        !self.live.is_empty()
    }

    /// The one in front: an answer it waits for first (read back, a choice),
    /// else the latest counting down.
    pub fn front(&self) -> Option<&Live> {
        let answering = |l: &&Live| matches!(l.stage, Stage::ReadBack | Stage::Which);
        self.live.iter().filter(answering).max_by_key(|l| l.id).or_else(|| self.live.iter().max_by_key(|l| l.id))
    }

    pub fn got_log(&mut self, entries: Vec<Entry>) {
        self.log = Some(entries);
    }

    /// The log, once, for its sheet.
    pub fn take_log(&mut self) -> Option<Vec<Entry>> {
        self.log.take()
    }
}

/// How the person's message came in, as the composer knows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Said {
    pub dictated: bool,
    pub pasted: bool,
}

/// What the composer sends for the person's message: to a daemon that reads
/// words, `say` in place of `submit` (a memo's too, `later`) — it raises any
/// card itself, so none is raised here; to one from before, as it was.
pub fn commands(commands: Vec<Command>, squad: &str, read: bool, said: Said) -> Vec<Command> {
    if !read {
        return commands;
    }
    let via = if said.dictated { Via::Dictated } else { Via::Typed };
    let say = |session, text, later, about| Command::Say { squad: squad.to_owned(), session, text, via, all: false, later, pasted: said.pasted, about, device: None };
    commands
        .into_iter()
        .filter(|c| !matches!(c, Command::ProposeAccept { .. }))
        .map(|c| match c {
            Command::Submit { session, text, about } => say(session, text, false, about),
            Command::SubmitLater { session, text, about } => say(session, text, true, about),
            other => other,
        })
        .collect()
}

// ── the card over the composer ──

/// Its heading: what kind of wait it is.
pub fn heading(live: &Live) -> &'static str {
    match live.stage {
        Stage::Undo => t!("words.head_undo"),
        Stage::Which => t!("words.head_which"),
        Stage::ReadBack => t!("words.head_read_back"),
        Stage::Countdown => t!("words.head_countdown"),
    }
}

/// Seconds left on its window, rounded up — none past it.
pub fn seconds_left(live: &Live, now_ms: u64) -> u64 {
    live.until.saturating_sub(now_ms).div_ceil(1000)
}

/// The line under it: what answers it — and, asked again, that it was.
pub fn hint(live: &Live, now_ms: u64) -> String {
    let seconds = seconds_left(live, now_ms).to_string();
    if live.asked_again > 0 && matches!(live.stage, Stage::ReadBack | Stage::Which) {
        return t!("words.hint_again", n = live.asked_again.to_string());
    }
    match live.stage {
        Stage::Undo => t!("words.hint_undo", seconds = seconds),
        Stage::Countdown => t!("words.hint_countdown", seconds = seconds),
        Stage::ReadBack if live.tier == Tier::Outward => t!("words.hint_read_back_outward").to_owned(),
        Stage::ReadBack => t!("words.hint_read_back").to_owned(),
        Stage::Which => t!("words.hint_which").to_owned(),
    }
}

/// Its button: Undo, Cancel, or — read back — leave it on its card.
pub fn button(live: &Live) -> &'static str {
    match live.stage {
        Stage::Undo => t!("words.undo"),
        Stage::Countdown => t!("words.cancel"),
        Stage::ReadBack | Stage::Which => t!("words.leave"),
    }
}

/// How much is at stake, in a word.
pub fn tier(tier: Tier) -> &'static str {
    match tier {
        Tier::Low => t!("words.tier_low"),
        Tier::High => t!("words.tier_high"),
        Tier::Outward => t!("words.tier_outward"),
    }
}

pub fn via(via: Via) -> &'static str {
    match via {
        Via::Typed => t!("words.via_typed"),
        Via::Dictated => t!("words.via_dictated"),
        Via::Voice => t!("words.via_voice"),
    }
}

// ── where words went wrong ──

pub fn outcome(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => t!("words.outcome_done"),
        Outcome::Undone => t!("words.outcome_undone"),
        Outcome::Cancelled => t!("words.outcome_cancelled"),
        Outcome::Held => t!("words.outcome_held"),
        Outcome::Lapsed => t!("words.outcome_lapsed"),
        Outcome::Moot => t!("words.outcome_moot"),
        Outcome::NeedsCard => t!("words.outcome_needs_card"),
        Outcome::Dropped => t!("words.outcome_dropped"),
    }
}

pub fn flag(flag: Flag) -> &'static str {
    match flag {
        Flag::Undone => t!("words.flag_undone"),
        Flag::Cancelled => t!("words.flag_cancelled"),
        Flag::AskedAgain => t!("words.flag_asked_again"),
        Flag::RevertedLater => t!("words.flag_reverted_later"),
        Flag::Ambiguous => t!("words.flag_ambiguous"),
        Flag::Held => t!("words.flag_held"),
        Flag::Lapsed => t!("words.flag_lapsed"),
        Flag::Dropped => t!("words.flag_dropped"),
    }
}

fn read_as(read: Reply) -> &'static str {
    match read {
        Reply::Yes => t!("words.read_yes"),
        Reply::No => t!("words.read_no"),
        Reply::Unclear => t!("words.read_unclear"),
        Reply::Cancel => t!("words.read_cancel"),
        Reply::Undo => t!("words.read_undo"),
        Reply::Pick => t!("words.read_pick"),
        Reply::Other => t!("words.read_other"),
    }
}

/// `2026-11-06 14:30` of a unix-ms time, the Mac's local time.
fn clock(ms: u64) -> String {
    crate::compare::local_date(ms / 1000)
}

/// One entry, as the log shows it: when and how, then what was said, read
/// back, answered, how it ended, and its flags.
pub fn entry_row(e: &Entry) -> Row {
    let mut head = format!("{} · {}", clock(e.said_at), via(e.via));
    if !e.flags.is_empty() {
        head = format!("⚑ {head}");
    }
    let mut parts = vec![t!("words.log_said", said = e.said.clone())];
    if !e.read_back.is_empty() {
        parts.push(t!("words.log_read_back", what = e.read_back.clone()));
    }
    for q in &e.quotes {
        parts.push(t!("words.quote_line", member = q.by.clone(), text = q.text.clone()));
    }
    for answer in &e.answers {
        parts.push(t!("words.log_answer", words = answer.words.clone(), read = read_as(answer.read)));
    }
    let took = e.decided_at.map(|at| at.saturating_sub(e.read_back_at.unwrap_or(e.said_at)));
    let mut end = format!("{} · {}", outcome(e.outcome), tier(e.tier));
    if let Some(ms) = took.filter(|_| !e.read_back.is_empty()) {
        end.push_str(&format!(" · {}", t!("words.log_took", seconds = (ms / 1000).to_string())));
    }
    parts.push(end);
    if !e.flags.is_empty() {
        parts.push(e.flags.iter().map(|f| flag(*f)).collect::<Vec<_>>().join(", "));
    }
    Row::text(head, parts.join(" → "))
}

/// What deleting the log asks first.
pub fn delete_ask(squad: &str) -> Ask {
    Ask { title: t!("words.delete_title").to_owned(), body: vec![t!("words.delete_body").to_owned()], yes: t!("words.delete_yes").to_owned(), then: Then::Send(vec![Command::ClearWordsLog { squad: squad.to_owned() }]) }
}

/// The sheet: flagged first — where to look — then the rest, newest first;
/// and, when there is anything, the way to delete it.
pub fn log_sheet(entries: &[Entry], squad: &str) -> Sheet {
    let (flagged, fine): (Vec<&Entry>, Vec<&Entry>) = entries.iter().partition(|e| !e.flags.is_empty());
    let mut rows = Vec::new();
    if entries.is_empty() {
        rows.push(Row::text("", t!("words.log_empty").to_owned()));
    }
    if !flagged.is_empty() {
        rows.push(Row { heading: true, ..Row::text("", t!("words.log_flagged", n = flagged.len().to_string())) });
        rows.extend(flagged.into_iter().map(entry_row));
    }
    if !fine.is_empty() {
        rows.push(Row { heading: true, ..Row::text("", t!("words.log_fine", n = fine.len().to_string())) });
        rows.extend(fine.into_iter().map(entry_row));
    }
    let buttons = if entries.is_empty() { Vec::new() } else { vec![Button::Ask(t!("words.delete").to_owned(), Box::new(delete_ask(squad)))] };
    Sheet { title: t!("words.log_title").to_owned(), lead: Some(t!("words.log_lead").to_owned()), rows, buttons, foot: Some(t!("words.log_foot").to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::accept::Verdict;
    use equipe_core::protocol::by_words::{Heard, Kind, Quote};

    fn live(id: u64, stage: Stage) -> Live {
        Live { id, session: 1, stage, tier: Tier::High, kind: Kind::Merge, read_back: "Merge T-1".into(), items: vec![], quotes: vec![], until: 10_000, said: "merge T-1".into(), via: Via::Typed, asked_again: 0, device: None }
    }

    #[test]
    fn what_waits_in_words_follows_the_daemon_and_an_answer_it_waits_for_comes_first() {
        let mut words = Words::default();
        words.load(None, false);
        assert!(!words.read, "a daemon from before reads no words");
        words.load(Some(vec![live(1, Stage::Undo)]), true);
        assert!(words.read && words.any() && words.all);
        words.live(live(2, Stage::ReadBack));
        words.live(live(3, Stage::Countdown));
        assert_eq!(words.front().map(|l| l.id), Some(2), "the read-back waits for the next message");
        words.live(Live { stage: Stage::Countdown, ..live(2, Stage::ReadBack) });
        assert_eq!(words.front().map(|l| l.id), Some(3), "none waits for an answer: the latest");
        words.close(3);
        words.close(2);
        words.close(1);
        assert!(!words.any() && words.front().is_none());
        words.got_log(vec![]);
        assert_eq!(words.take_log(), Some(vec![]));
        assert_eq!(words.take_log(), None, "shown once");
    }

    #[test]
    fn the_persons_message_goes_as_say_to_a_daemon_that_reads_words_and_as_before_to_one_that_doesnt() {
        let raise = Command::ProposeAccept { squad: "acme".into(), session: 1, verdict: Verdict::Accept, tickets: vec![1], note: String::new() };
        let submit = Command::Submit { session: 1, text: "T-1 수락".into(), about: Some(None) };
        let memo = Command::SubmitLater { session: 2, text: "ㄱㄱ".into(), about: None };
        let new = Command::NewConversation { squad: "acme".into(), session: 1, project: None };
        let old = vec![raise.clone(), submit.clone()];
        assert_eq!(commands(old.clone(), "acme", false, Said::default()), old, "a daemon from before: as it was");
        let said = Said { dictated: true, pasted: false };
        assert_eq!(
            commands(vec![new.clone(), raise, submit, memo], "acme", true, said),
            vec![
                new,
                Command::Say { squad: "acme".into(), session: 1, text: "T-1 수락".into(), via: Via::Dictated, all: false, later: false, pasted: false, about: Some(None), device: None },
                Command::Say { squad: "acme".into(), session: 2, text: "ㄱㄱ".into(), via: Via::Dictated, all: false, later: true, pasted: false, about: None, device: None },
            ],
            "no card raised here: the daemon does, from the words; all by words is the daemon's to know"
        );
        let pasted = commands(vec![Command::Submit { session: 1, text: "merge T-1".into(), about: None }], "acme", true, Said { pasted: true, ..Said::default() });
        assert!(matches!(&pasted[..], [Command::Say { pasted: true, via: Via::Typed, .. }]));
    }

    #[test]
    fn the_card_says_what_answers_it_and_counts_its_window_down() {
        assert_eq!(seconds_left(&live(1, Stage::Undo), 6_100), 4);
        assert_eq!(seconds_left(&live(1, Stage::Undo), 11_000), 0);
        assert_eq!(hint(&live(1, Stage::Undo), 6_100), "4초 안에 ‘되돌려’라고 하거나 되돌리기를 누르면 멈춰요");
        assert_eq!(hint(&live(1, Stage::ReadBack), 0), "다음 메시지가 답이에요: 진행하려면 ‘네’, 카드로 남기려면 ‘아니’");
        assert_eq!(button(&live(1, Stage::Countdown)), "취소");
        assert_eq!(hint(&Live { asked_again: 1, ..live(1, Stage::ReadBack) }, 0), "다시 물었어요 · 1번 — 진행하려면 ‘네’, 카드로 남기려면 ‘아니’");
        assert_eq!(heading(&live(1, Stage::ReadBack)), "말로 결정 · 확인해 주세요");
    }

    #[test]
    fn the_log_puts_where_words_went_wrong_first() {
        let entry = |id: u64, outcome: Outcome, flags: Vec<Flag>| Entry {
            id,
            session: 1,
            kind: Kind::Merge,
            tier: Tier::High,
            via: Via::Voice,
            all_by_words: true,
            said: "merge T-1".into(),
            read_back: "Merge T-1 into main, on this Mac".into(),
            items: vec![],
            quotes: vec![Quote { by: "pm".into(), text: "급해요".into() }],
            answers: vec![Heard { words: "음".into(), read: Reply::Unclear, at: 2_000, via: Via::Voice }, Heard { words: "ㄱㄱ".into(), read: Reply::Yes, at: 9_000, via: Via::Voice }],
            tickets: vec![1],
            said_at: 1_000,
            read_back_at: Some(1_000),
            decided_at: Some(9_000),
            closed_at: 9_000,
            outcome,
            asked_again: 1,
            chose: false,
            device: None,
            flags,
        };
        let sheet = log_sheet(&[entry(2, Outcome::Done, vec![]), entry(1, Outcome::Done, vec![Flag::AskedAgain, Flag::RevertedLater])], "acme");
        assert_eq!(sheet.title, "말이 어긋난 곳");
        assert!(sheet.rows[0].heading && sheet.rows[1].head.starts_with("⚑ ") && sheet.rows[1].head.ends_with("목소리"));
        assert_eq!(
            sheet.rows[1].text,
            "말한 것 “merge T-1” → 읽어 준 것: Merge T-1 into main, on this Mac → pm이 쓴 말: “급해요” → “음” (알아듣지 못함) → “ㄱㄱ” (네) → 했어요 · 확인이 필요해요 · 8초 걸림 → 다시 물음, 나중에 되돌림"
        );
        assert!(sheet.rows[2].heading && !sheet.rows[3].head.starts_with('⚑'));
        assert_eq!(log_sheet(&[], "acme").rows[0].text, "아직 말로 정한 게 없어요.");
        assert!(log_sheet(&[], "acme").buttons.is_empty(), "nothing to delete");
        let Button::Ask(label, ask) = &sheet.buttons[0] else { panic!("{:?}", sheet.buttons) };
        assert_eq!(label, "이 기록 지우기");
        assert_eq!(ask.then, Then::Send(vec![Command::ClearWordsLog { squad: "acme".into() }]));
        assert!(ask.destructive(), "red: it can't be taken back");
    }
}
