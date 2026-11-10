//! What each taste note was learned from, for the person to check it
//! against: the choices and send-backs on the tickets it names — its
//! source's, and the others it was seen on. A choice brings its picture and
//! the person's words; a send-back, its words. Linked by ticket, never by
//! guessing at the words, so what is shown is what happened. Pure.

use crate::protocol::TicketId;
use crate::protocol::design::{Decision, Evidence, EvidenceKind, TasteNote};

use super::notes::is_date;

/// The most shown for one note.
pub const MAX: usize = 12;

/// A decision as evidence: the question and what was chosen, its picture, why.
pub fn of_decision(d: &Decision) -> Evidence {
    let what = if d.question.trim().is_empty() { d.chosen.clone() } else { format!("{} \u{2192} {}", d.question.trim(), d.chosen.trim()) };
    Evidence { date: d.date.clone(), ticket: d.ticket, kind: EvidenceKind::Chose, what, words: d.why.clone(), picture: d.picture.clone() }
}

/// The send-backs equipe kept in the squad's memory (`design taste`):
/// `- Sent back “Pricing page”: “it looks cheap”` under its `## date · T-n` entry.
pub fn sent_backs(body: &str) -> Vec<Evidence> {
    let unquote = |s: &str| s.trim().trim_matches(['\u{201c}', '\u{201d}', '"']).trim().to_owned();
    let (mut date, mut ticket): (String, Option<TicketId>) = (String::new(), None);
    let mut out = Vec::new();
    for line in body.lines().map(str::trim) {
        if let Some(head) = line.strip_prefix("## ") {
            date = head.get(..10).filter(|d| is_date(d)).unwrap_or_default().to_owned();
            ticket = head.find("T-").and_then(|at| head[at + 2..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
            continue;
        }
        let Some(rest) = line.strip_prefix("- Sent back ") else { continue };
        let (title, words) = rest.split_once("\u{201d}: ").map_or((rest, ""), |(t, w)| (t, w));
        out.push(Evidence { date: date.clone(), ticket, kind: EvidenceKind::SentBack, what: unquote(title), words: Some(unquote(words)).filter(|w| !w.is_empty()), picture: None });
    }
    out
}

/// Each note given what it was learned from, newest first: the choices and
/// send-backs on its tickets.
pub fn link(notes: &mut [TasteNote], decisions: &[Decision], sent_back: &[Evidence]) {
    let all: Vec<Evidence> = decisions.iter().map(of_decision).chain(sent_back.iter().cloned()).collect();
    for note in notes.iter_mut() {
        let tickets = note.tickets();
        let mut found: Vec<Evidence> = Vec::new();
        for e in all.iter().filter(|e| e.ticket.is_some_and(|t| tickets.contains(&t))) {
            if !found.iter().any(|f| f.kind == e.kind && f.ticket == e.ticket && f.what == e.what) {
                found.push(e.clone());
            }
        }
        found.sort_by(|a, b| b.date.cmp(&a.date));
        found.truncate(MAX);
        note.evidence = found;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::design::TasteFrom;

    fn decision(date: &str, question: &str, chosen: &str, ticket: Option<u32>) -> Decision {
        Decision { date: date.into(), question: question.into(), chosen: chosen.into(), ticket, ..Decision::default() }
    }

    #[test]
    fn a_send_back_kept_in_the_memory_is_evidence_with_the_persons_words() {
        let body = "## 2026-11-03 · T-336 (designer)\n- Sent back \u{201c}Pricing page\u{201d}: \u{201c}채도가 너무 높아\u{201d}\n\n## 2026-11-01 · T-272 (designer)\n- Which hero? \u{2192} chose B";
        let found = sent_backs(body);
        assert_eq!(found, [Evidence { date: "2026-11-03".into(), ticket: Some(336), kind: EvidenceKind::SentBack, what: "Pricing page".into(), words: Some("채도가 너무 높아".into()), picture: None }]);
    }

    #[test]
    fn each_note_is_linked_to_what_happened_on_its_tickets_and_nothing_else() {
        let mut picked = decision("2026-11-01", "히어로", "B — 여백 넉넉히", Some(272));
        picked.why = Some("숨 쉬게".into());
        picked.picture = Some("/d/decisions/hero.png".into());
        let decisions = vec![picked, decision("2026-11-02", "강조 색", "Clay", Some(336)), decision("2026-10-01", "로고", "점", Some(144)), decision("2026-10-02", "아무 티켓 없음", "x", None)];
        let sent = vec![Evidence { date: "2026-11-03".into(), ticket: Some(336), kind: EvidenceKind::SentBack, what: "Pricing".into(), words: Some("너무 쨍해".into()), picture: None }];
        let note = |text: &str, from: TasteFrom, seen: Vec<u32>| TasteNote { text: text.into(), from, seen, ..TasteNote::default() };
        let mut notes = vec![
            note("채도 높은 파랑은 싫어함", TasteFrom::SentBack { ticket: 336 }, vec![]),
            note("여백은 넉넉하게", TasteFrom::Noticed { ticket: Some(301) }, vec![272]),
            note("모서리는 둥글게", TasteFrom::Person, vec![]),
        ];
        link(&mut notes, &decisions, &sent);
        // Its ticket's send-back and choice, newest first.
        let first: Vec<(&str, EvidenceKind)> = notes[0].evidence.iter().map(|e| (e.date.as_str(), e.kind)).collect();
        assert_eq!(first, [("2026-11-03", EvidenceKind::SentBack), ("2026-11-02", EvidenceKind::Chose)]);
        assert_eq!(notes[0].evidence[0].words.as_deref(), Some("너무 쨍해"));
        // A pattern: what it was seen on, with the picture and the person's words.
        assert_eq!(notes[1].evidence.len(), 1);
        let e = &notes[1].evidence[0];
        assert_eq!((e.what.as_str(), e.words.as_deref(), e.picture.as_deref(), e.ticket), ("히어로 \u{2192} B — 여백 넉넉히", Some("숨 쉬게"), Some("/d/decisions/hero.png"), Some(272)));
        // The person's own names no ticket: nothing guessed for it.
        assert!(notes[2].evidence.is_empty());
        // The same choice read from two places is shown once.
        let twice = [decisions[1].clone(), decisions[1].clone()];
        link(&mut notes, &twice, &[]);
        assert_eq!(notes[0].evidence.len(), 1);
    }
}
