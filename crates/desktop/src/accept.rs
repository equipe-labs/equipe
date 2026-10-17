//! Handed-in work the person said in words to accept, merge or send back —
//! "전부 수락", "T-1이랑 T-3 수락해줘", "머지까지 해줘" — as the card they
//! confirm it on: above the composer in the conversation it was said in, and
//! in 나에게 under "결정 필요". Which cards wait and which tickets the person
//! kept on each, what a card says (its question, each ticket, what starts
//! after, how code lands, the words on its button), and the plain reading of
//! the person's own words that raises one before any member answers. No UI
//! types here, so the rules are tested without a window.
//!
//! Nothing here accepts: the card's button sends the person's command
//! (`ConfirmAccept`), and the daemon then sends what the board's buttons send.
//! Reading the words only raises the card (`ProposeAccept`) — a fixed
//! pattern, not an agent, and never more than a card.

use std::collections::HashMap;

use equipe_core::accept::{all_of, starts_after, unfit};
use equipe_core::protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};
use equipe_core::protocol::{Command, SessionId, Ticket, TicketId};
use equipe_core::t;

use crate::board::{MergeWay, needs_way};

// ── the cards waiting ──

/// Cards waiting for the person, oldest first, and the tickets they took off
/// each. One decided here hides at once, so a second click can't send it
/// twice; the daemon's `AcceptClosed` takes it for good.
#[derive(Debug, Clone, Default)]
pub struct Cards {
    cards: Vec<AcceptProposal>,
    sent: Vec<u64>,
    /// Per card, the tickets left out (their box unticked).
    left_out: HashMap<u64, Vec<TicketId>>,
}

impl Cards {
    /// The snapshot's cards replace everything.
    pub fn load(&mut self, cards: Vec<AcceptProposal>) {
        self.left_out.retain(|id, _| cards.iter().any(|c| c.id == *id));
        self.cards = cards;
        self.sent.clear();
    }

    /// A card put up. Whether it is new here.
    pub fn push(&mut self, card: AcceptProposal) -> bool {
        if self.cards.iter().any(|c| c.id == card.id) {
            return false;
        }
        self.cards.push(card);
        true
    }

    /// No longer waiting, whoever decided.
    pub fn close(&mut self, id: u64) {
        self.cards.retain(|c| c.id != id);
        self.sent.retain(|s| *s != id);
        self.left_out.remove(&id);
    }

    /// What was sent into a dead link never came back: show it again.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
    }

    /// Every card still waiting, oldest first.
    pub fn open(&self) -> impl Iterator<Item = &AcceptProposal> {
        self.cards.iter().filter(|c| !self.sent.contains(&c.id))
    }

    pub fn get(&self, id: u64) -> Option<&AcceptProposal> {
        self.open().find(|c| c.id == id)
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open().map(|c| c.id).collect()
    }

    /// The oldest card said in conversation `session`.
    pub fn shown_in(&self, session: SessionId) -> Option<&AcceptProposal> {
        self.open().find(|c| c.session == session)
    }

    /// Whether `ticket` is ticked on card `id`.
    pub fn keeps(&self, id: u64, ticket: TicketId) -> bool {
        !self.left_out.get(&id).is_some_and(|l| l.contains(&ticket))
    }

    /// Tick or untick a ticket's box on a card.
    pub fn toggle(&mut self, id: u64, ticket: TicketId) {
        let list = self.left_out.entry(id).or_default();
        match list.iter().position(|t| *t == ticket) {
            Some(at) => {
                list.remove(at);
            }
            None => list.push(ticket),
        }
    }

    /// The tickets still ticked on card `id`, in its order.
    pub fn kept(&self, id: u64) -> Vec<TicketId> {
        self.get(id).map(|c| c.tickets.iter().copied().filter(|t| self.keeps(id, *t)).collect()).unwrap_or_default()
    }

    /// The person's answer to card `id`: the command to send, the card hidden
    /// at once. `land` says how each ticket lands once accepted (the
    /// project's way). `None` when it no longer waits — or, confirming,
    /// when no ticket is ticked.
    pub fn decide(&mut self, squad: &str, id: u64, confirm: bool, land: impl Fn(TicketId) -> Option<Land>) -> Option<Command> {
        self.get(id)?;
        let squad = squad.to_owned();
        let command = if confirm {
            let kept = self.kept(id);
            if kept.is_empty() {
                return None;
            }
            Command::ConfirmAccept { squad, id, tickets: kept.into_iter().map(|ticket| Confirmed { ticket, merge: land(ticket) }).collect() }
        } else {
            Command::DeclineAccept { squad, id }
        };
        self.sent.push(id);
        Some(command)
    }
}

/// How a confirmed ticket lands, by the project's way (`way`) — as A on the
/// board lands it: code merged (and pushed) when the project says so; asked
/// to merge, merged at least. A task's work goes into the squad's folder with
/// the acceptance; sending back lands nothing.
pub fn landing(t: &Ticket, verdict: Verdict, way: Option<MergeWay>) -> Option<Land> {
    if verdict == Verdict::Changes || !(needs_way(t) || t.not_on_main()) {
        return None;
    }
    match (way, verdict) {
        (Some(MergeWay::Push), _) => Some(Land::Push),
        (Some(MergeWay::Merge), _) | (_, Verdict::Merge) => Some(Land::Merge),
        _ => None,
    }
}

// ── the card's words ──

/// The tickets, as the card names them: "T-1, T-3".
pub fn list(ids: &[TicketId]) -> String {
    ids.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ")
}

/// The card's question: "T-1, T-3 수락할까요?".
pub fn title(card: &AcceptProposal) -> String {
    let tickets = list(&card.tickets);
    match card.verdict {
        Verdict::Accept => t!("accept.ask", tickets = tickets),
        Verdict::Merge => t!("accept.ask_merge", tickets = tickets),
        Verdict::Changes => t!("accept.ask_changes", tickets = tickets),
    }
}

/// The card's main button, for the tickets ticked of all it names: "수락하기",
/// "모두 수락하기", "2개 수락하기".
pub fn yes_label(card: &AcceptProposal, kept: usize) -> String {
    let all = card.tickets.len();
    let key = match (card.verdict, all > 1 && kept == all) {
        (Verdict::Accept, false) => "accept.yes",
        (Verdict::Accept, true) => "accept.yes_all",
        (Verdict::Merge, false) => "accept.yes_merge",
        (Verdict::Merge, true) => "accept.yes_merge_all",
        (Verdict::Changes, false) => "accept.yes_changes",
        (Verdict::Changes, true) => "accept.yes_changes_all",
    };
    if all > 1 && kept < all && kept > 0 {
        let some = match card.verdict {
            Verdict::Accept => "accept.yes_some",
            Verdict::Merge => "accept.yes_merge_some",
            Verdict::Changes => "accept.yes_changes_some",
        };
        return crate::i18n::catalog().fill(some, &[("count", kept.to_string())]);
    }
    crate::i18n::catalog().get(key).to_owned()
}

/// What starts once the ticked tickets are accepted: "수락하면 T-45 작업이
/// 시작돼요". `None` when nothing waits on them, or they go back.
pub fn starts(card: &AcceptProposal, kept: &[TicketId], tickets: &[Ticket]) -> Option<String> {
    if card.verdict == Verdict::Changes {
        return None;
    }
    let next = starts_after(tickets, kept);
    (!next.is_empty()).then(|| t!("accept.starts", tickets = list(&next)))
}

/// How the ticked code lands, by project: "web — main에 병합+push". `None`
/// when nothing lands (tasks, or sending back).
pub fn lands(card: &AcceptProposal, kept: &[TicketId], tickets: &[Ticket], project: impl Fn(&Ticket) -> String, way: impl Fn(&str) -> Option<MergeWay>) -> Option<String> {
    if card.verdict == Verdict::Changes {
        return None;
    }
    let mut parts: Vec<(String, String)> = Vec::new();
    for t in kept.iter().filter_map(|id| tickets.iter().find(|t| t.id == *id)) {
        if !(needs_way(t) || t.not_on_main()) {
            continue;
        }
        let name = project(t);
        let words = match landing(t, card.verdict, way(&name)) {
            Some(Land::Push) => MergeWay::Push.label(),
            Some(Land::Merge) => MergeWay::Merge.label(),
            None => MergeWay::Branch.label(),
        };
        if !parts.iter().any(|(p, _)| *p == name) {
            parts.push((name, words.to_owned()));
        }
    }
    // One way for every project: said once.
    if !parts.is_empty() && parts.iter().all(|(_, w)| *w == parts[0].1) {
        return Some(parts.swap_remove(0).1);
    }
    (!parts.is_empty()).then(|| parts.into_iter().map(|(p, w)| format!("{p} — {w}")).collect::<Vec<_>>().join(" · "))
}

// ── the person's own words ──

/// The person's words, read as plainly accepting or merging handed-in work:
/// the verdict, and the tickets — named (`T-1`), or all of them ("전부",
/// "다", "all"). Anything less plain — a question, a "not", an "if", no
/// tickets, sending back (which needs what to change) — is left to the
/// member they said it to.
pub fn read_words(text: &str, tickets: &[Ticket]) -> Option<(Verdict, Vec<TicketId>)> {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() || lower.chars().count() > 60 || lower.starts_with('/') || lower.starts_with('@') {
        return None;
    }
    // Asking, not saying; a "not", a "wait", an "if": not plain.
    const UNSURE: [&str; 30] = [
        "?", "？", "않", "안 ", "안해", "안 해", "말고", "말아", "마세요", "하지 마", "하지마", "지 마", "아직", "보류", "잠깐", "나중", "하면", "되면", "이면", "으면",
        "not ", "n't", "dont", "never", "wait", "later", "hold", "if ", "unless", "after ",
    ];
    if UNSURE.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let merge = ["머지", "병합", "merge"].iter().any(|w| lower.contains(w));
    let accept = ["수락", "승인", "accept", "approve"].iter().any(|w| lower.contains(w));
    if !(merge || accept) {
        return None;
    }
    let verdict = if merge { Verdict::Merge } else { Verdict::Accept };
    let named = named_tickets(&lower);
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '~')).filter(|w| !w.is_empty()).collect();
    let verbs = ["수락", "승인", "머지", "병합"];
    let all = words.iter().any(|w| {
        w.starts_with("전부") || w.starts_with("모두") || w.starts_with("몽땅") || matches!(*w, "다" | "all" | "everything") || verbs.iter().any(|v| w.starts_with(&format!("다{v}")))
    });
    let ids = match (named.is_empty(), all) {
        (false, _) => named,
        (true, true) => all_of(tickets, verdict),
        (true, false) => return None,
    };
    // Only what can be put to the person: else the member answers.
    let fit: Vec<TicketId> = ids.into_iter().filter(|id| tickets.iter().any(|t| t.id == *id && unfit(t, verdict).is_none())).collect();
    (!fit.is_empty()).then_some((verdict, fit))
}

/// `T-1`, `t-42` in the text, in order, once each.
fn named_tickets(lower: &str) -> Vec<TicketId> {
    let mut out = Vec::new();
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i + 2 < chars.len() + 1 {
        let boundary = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        if boundary && chars[i] == 't' && chars.get(i + 1) == Some(&'-') {
            let digits: String = chars[i + 2..].iter().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(id) = digits.parse::<TicketId>()
                && !out.contains(&id)
            {
                out.push(id);
            }
            i += 2 + digits.len();
            continue;
        }
        i += 1;
    }
    out
}

/// The command that raises the card for words read so, in conversation
/// `session` — sent before the words themselves, so the member there, the
/// PM included, finds it up and raises nothing twice.
pub fn raise(squad: &str, session: SessionId, text: &str, tickets: &[Ticket]) -> Option<Command> {
    let (verdict, tickets) = read_words(text, tickets)?;
    Some(Command::ProposeAccept { squad: squad.to_owned(), session, verdict, tickets, note: String::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::TicketStatus;

    fn ticket(id: TicketId, status: &str, depends_on: &[TicketId]) -> Ticket {
        serde_json::from_value(serde_json::json!({ "id": id, "title": format!("work {id}"), "body": "", "status": status, "kind": "code", "branch": format!("equipe/t-{id}"), "depends_on": depends_on, "created_by": "pm" })).unwrap()
    }

    fn board() -> Vec<Ticket> {
        let mut off = ticket(6, "done", &[]);
        off.merged = Some(false);
        vec![ticket(1, "review", &[]), ticket(2, "in_progress", &[]), ticket(3, "review", &[]), ticket(4, "todo", &[1, 3]), ticket(5, "todo", &[1]), off]
    }

    fn card(id: u64, tickets: &[TicketId]) -> AcceptProposal {
        AcceptProposal { id, verdict: Verdict::Accept, tickets: tickets.to_vec(), note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 }
    }

    #[test]
    fn a_card_confirms_the_tickets_left_ticked_once_and_hides_at_once() {
        let mut cards = Cards::default();
        cards.load(vec![card(7, &[1, 3]), AcceptProposal { session: 2, ..card(8, &[1]) }]);
        assert!(!cards.push(card(7, &[1, 3])), "not twice");
        assert_eq!(cards.shown_in(1).map(|c| c.id), Some(7));
        assert_eq!(cards.shown_in(2).map(|c| c.id), Some(8));
        // Untick T-3: only T-1 goes, merged as its project says.
        cards.toggle(7, 3);
        assert!(!cards.keeps(7, 3) && cards.keeps(7, 1));
        assert_eq!(cards.kept(7), [1]);
        let land = |t: TicketId| (t == 1).then_some(Land::Push);
        assert_eq!(cards.decide("acme", 7, true, land), Some(Command::ConfirmAccept { squad: "acme".into(), id: 7, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Push) }] }));
        assert_eq!(cards.decide("acme", 7, true, land), None, "hidden: no second answer");
        assert_eq!(cards.ids(), [8]);
        // Nothing ticked: nothing to confirm, and the card stays.
        cards.toggle(8, 1);
        assert_eq!(cards.decide("acme", 8, true, |_| None), None);
        cards.toggle(8, 1);
        assert_eq!(cards.decide("acme", 8, false, |_| None), Some(Command::DeclineAccept { squad: "acme".into(), id: 8 }));
        assert!(cards.ids().is_empty());
        // A lost link shows what never came back; the daemon's word takes it for good.
        cards.forget_sent();
        assert_eq!(cards.ids(), [7, 8]);
        assert_eq!(cards.kept(7), [1], "what was unticked stays so");
        cards.close(7);
        assert_eq!(cards.ids(), [8]);
    }

    #[test]
    fn code_lands_the_projects_way_and_to_merge_it_merges_at_least() {
        let review = ticket(1, "review", &[]);
        assert_eq!(landing(&review, Verdict::Accept, Some(MergeWay::Push)), Some(Land::Push));
        assert_eq!(landing(&review, Verdict::Accept, Some(MergeWay::Merge)), Some(Land::Merge));
        assert_eq!(landing(&review, Verdict::Accept, Some(MergeWay::Branch)), None);
        assert_eq!(landing(&review, Verdict::Accept, None), None, "no way yet: accepted, left on its branch");
        assert_eq!(landing(&review, Verdict::Merge, None), Some(Land::Merge));
        assert_eq!(landing(&review, Verdict::Merge, Some(MergeWay::Branch)), Some(Land::Merge), "they asked to merge");
        assert_eq!(landing(&review, Verdict::Changes, Some(MergeWay::Push)), None);
        let mut task = review.clone();
        task.kind = equipe_core::protocol::TicketKind::Task;
        assert_eq!(landing(&task, Verdict::Merge, Some(MergeWay::Push)), None, "a task's work goes into the folder");
    }

    #[test]
    fn the_card_says_what_it_asks_what_starts_after_and_how_code_lands() {
        let both = card(7, &[1, 3]);
        assert_eq!(title(&both), "T-1, T-3 수락할까요?");
        assert_eq!(title(&AcceptProposal { verdict: Verdict::Merge, ..both.clone() }), "T-1, T-3 수락하고 머지할까요?");
        assert_eq!(title(&AcceptProposal { verdict: Verdict::Changes, ..both.clone() }), "T-1, T-3 반려할까요?");
        assert_eq!(yes_label(&both, 2), "모두 수락하기");
        assert_eq!(yes_label(&both, 1), "1개 수락하기");
        assert_eq!(yes_label(&card(7, &[1]), 1), "수락하기");
        assert_eq!(yes_label(&AcceptProposal { verdict: Verdict::Merge, ..card(7, &[1]) }, 1), "수락하고 머지");
        assert_eq!(starts(&both, &[1, 3], &board()).as_deref(), Some("수락하면 T-4, T-5 작업이 시작돼요"));
        assert_eq!(starts(&both, &[1], &board()).as_deref(), Some("수락하면 T-5 작업이 시작돼요"), "T-4 still waits on T-3");
        assert_eq!(starts(&both, &[3], &board()), None);
        let project = |t: &Ticket| if t.id == 1 { "web".to_owned() } else { "api".to_owned() };
        let way = |p: &str| (p == "web").then_some(MergeWay::Push);
        assert_eq!(lands(&both, &[1, 3], &board(), project, way).as_deref(), Some("web — 병합+push · api — 브랜치에 둠"));
        assert_eq!(lands(&both, &[1, 3], &board(), project, |_| None).as_deref(), Some("브랜치에 둠"), "one way for all: said once");
    }

    #[test]
    fn plain_words_raise_the_card_and_anything_less_plain_is_left_to_the_member() {
        let b = board();
        let read = |text: &str| read_words(text, &b);
        assert_eq!(read("전부다 수락할게."), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("다 수락"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("다수락해줘"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("모두 승인합니다"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("Accept them all!"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("accept all"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("T-1 수락"), Some((Verdict::Accept, vec![1])));
        assert_eq!(read("T-1이랑 T-3 수락해줘"), Some((Verdict::Accept, vec![1, 3])));
        assert_eq!(read("t-3, t-1 accept"), Some((Verdict::Accept, vec![3, 1])));
        assert_eq!(read("전부 머지까지 해줘"), Some((Verdict::Merge, vec![1, 3, 6])), "to merge, accepted work off main too");
        assert_eq!(read("T-1 수락하고 머지"), Some((Verdict::Merge, vec![1])));
        // Not handed in, or no such ticket: nothing to put up.
        assert_eq!(read("T-2 수락"), None);
        assert_eq!(read("T-1, T-2, T-99 수락"), Some((Verdict::Accept, vec![1])));
        for unsure in [
            "전부 수락할까?",
            "T-1 수락하지 마",
            "아직 수락하지 말아줘",
            "테스트 통과하면 다 수락할게",
            "수락할게",
            "수락 버튼 어디 있어",
            "T-1 수락 버튼이 안 눌려요",
            "이건 반려",
            "T-1 어때",
            "don't accept T-1",
            "accept all after the tests",
            "/accept T-1",
            "다 했어요",
            "오늘 한 일 정리해서 내일 아침에 다시 보여줘. 그리고 T-1은 리뷰가 끝나면 수락할 생각이에요, 문제 없으면요",
        ] {
            assert_eq!(read(unsure), None, "{unsure}");
        }
        assert_eq!(raise("acme", 1, "전부 수락", &b), Some(Command::ProposeAccept { squad: "acme".into(), session: 1, verdict: Verdict::Accept, tickets: vec![1, 3], note: String::new() }));
        assert!(b.iter().any(|t| t.status == TicketStatus::Review));
    }
}
