//! Design decisions as their files keep them, the brand's decision record:
//!
//! ```md
//! ## 2026-11-03 · 히어로 레이아웃
//! - Chose: B — 한 단, 여백 넉넉히
//! - Picture: decisions/2026-11-03-히어로-레이아웃.png
//! - Passed over: A — 두 단; C — 꽉 찬 사진
//! - Why: “좀 더 숨 쉬게”
//! - Ticket: T-272
//! ```
//!
//! One file each in `design/decisions/` (two writers — equipe on a pick, the
//! designer in a ticket's worktree — never touch the same file, so a merge
//! never conflicts); a `decisions.md` holding many reads the same, and so do
//! the entries the old `taste.md` template kept. What equipe learned in the
//! squad's memory (`design taste`: "Q → chose X; passed over …") is read as
//! decisions too. Pure: no disk here.

use crate::protocol::design::Decision;

use super::notes::is_date;

/// The most decisions sent to a screen, newest first.
pub const MAX: usize = 200;

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A decision as its own file.
pub fn text(d: &Decision) -> String {
    let mut out = format!("## {} · {}\n- Chose: {}\n", d.date, one_line(&d.question), one_line(&d.chosen));
    if let Some(picture) = &d.picture {
        out.push_str(&format!("- Picture: {}\n", one_line(picture)));
    }
    if !d.passed.is_empty() {
        out.push_str(&format!("- Passed over: {}\n", d.passed.iter().map(|p| one_line(p)).collect::<Vec<_>>().join("; ")));
    }
    if let Some(why) = d.why.as_deref().map(one_line).filter(|w| !w.is_empty()) {
        out.push_str(&format!("- Why: \u{201c}{why}\u{201d}\n"));
    }
    if let Some(t) = d.ticket {
        out.push_str(&format!("- Ticket: T-{t}\n"));
    }
    out
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches(|c| matches!(c, '"' | '\u{201c}' | '\u{201d}' | '\'')).trim().to_owned()
}

fn ticket_in(text: &str) -> Option<u32> {
    let at = text.find("T-")?;
    let digits: String = text[at + 2..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// A picture's path written in a choice, between backticks: `b.png`.
fn picture_in(text: &str) -> Option<(String, String)> {
    let mut parts = text.split('`');
    let before = parts.next()?;
    let path = parts.next()?;
    let after: String = parts.collect::<Vec<_>>().join("`");
    crate::visuals::is_image_name(path).then(|| {
        let rest = format!("{before}{after}").replace("()", "").replace("( )", "");
        (path.to_owned(), one_line(&rest))
    })
}

/// The decisions in a file, as it orders them.
pub fn parse(text: &str) -> Vec<Decision> {
    let mut out: Vec<Decision> = Vec::new();
    let mut current: Option<Decision> = None;
    let mut fence = false;
    let finish = |d: Option<Decision>, out: &mut Vec<Decision>| {
        if let Some(mut d) = d.filter(|d| !d.chosen.is_empty() || d.why.is_some()) {
            if d.picture.is_none()
                && let Some((path, rest)) = picture_in(&d.chosen)
            {
                d.picture = Some(path);
                d.chosen = rest;
            }
            out.push(d);
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            finish(current.take(), &mut out);
            let heading = heading.trim_start_matches('#').trim();
            if let Some(date) = heading.get(..10).filter(|d| is_date(d)) {
                let rest = heading[10..].trim_start_matches([' ', '·', '—', '-', ':']).trim();
                current = Some(Decision { date: date.to_owned(), question: rest.to_owned(), ..Decision::default() });
            }
            continue;
        }
        let Some(d) = current.as_mut() else { continue };
        let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) else { continue };
        let Some((key, value)) = item.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "chose" | "chosen" => d.chosen = value.to_owned(),
            "picture" => d.picture = Some(value.trim_matches('`').to_owned()).filter(|p| !p.is_empty()),
            "passed over" | "passed" => d.passed = value.split(';').map(str::trim).filter(|p| !p.is_empty() && !p.starts_with('(')).map(str::to_owned).collect(),
            "why" | "in their words" => d.why = Some(unquote(value)).filter(|w| !w.is_empty() && !w.starts_with('…')),
            "ticket" => d.ticket = ticket_in(value),
            _ => {}
        }
    }
    finish(current, &mut out);
    out
}

/// What equipe kept of the person's answers in the squad's memory
/// (`design taste`), as decisions: each "Q → chose X" line, and each
/// question answered in their own words alone. Send-backs aren't decisions.
pub fn from_history(body: &str) -> Vec<Decision> {
    let mut out = Vec::new();
    let (mut date, mut ticket) = (String::new(), None);
    for line in body.lines().map(str::trim) {
        if let Some(head) = line.strip_prefix("## ") {
            date = head.get(..10).filter(|d| is_date(d)).unwrap_or_default().to_owned();
            ticket = ticket_in(head);
            continue;
        }
        let Some(item) = line.strip_prefix("- ").filter(|_| !date.is_empty()) else { continue };
        if item.starts_with("Sent back ") {
            continue;
        }
        let (question, rest) = match item.split_once(" \u{2192} ") {
            Some((q, rest)) => (q, rest),
            None => match item.split_once("; in their words: ") {
                Some((q, words)) => (q, words),
                None => continue,
            },
        };
        let mut d = Decision { date: date.clone(), question: question.trim().to_owned(), ticket, ..Decision::default() };
        if item.contains(" \u{2192} ") {
            for part in rest.split("; ") {
                if let Some(chose) = part.strip_prefix("chose ") {
                    d.chosen = chose.trim_end_matches(" (the recommendation)").trim().to_owned();
                } else if let Some(passed) = part.strip_prefix("passed over ") {
                    d.passed = passed.split(", ").map(str::to_owned).collect();
                } else if let Some(words) = part.strip_prefix("in their words: ") {
                    d.why = Some(unquote(words));
                }
            }
        } else {
            // Answered in their own words alone: those are the choice.
            d.chosen = unquote(rest);
        }
        if !d.chosen.is_empty() {
            out.push(d);
        }
    }
    out
}

/// The visual choices in a set of answered questions, as decisions to
/// record, each with where its chosen picture is now: a question whose pick
/// has a picture. (One answered in words alone stays in what equipe learned.)
pub fn from_answers(set: &crate::protocol::decide::QuestionSet, answers: &[crate::protocol::decide::Answer], today: &str) -> Vec<(Decision, String)> {
    let label = |l: &str| l.trim().trim_end_matches("(Recommended)").trim().to_owned();
    set.questions
        .iter()
        .zip(answers)
        .filter_map(|(q, a)| {
            let chosen = q.options.get(*a.picked.first()?)?;
            let picture = chosen.images.first()?.clone();
            let passed = q.options.iter().enumerate().filter(|(i, _)| !a.picked.contains(i)).map(|(_, o)| label(&o.label)).collect();
            let chose: Vec<String> = a.picked.iter().filter_map(|i| q.options.get(*i)).map(|o| label(&o.label)).collect();
            let why = a.other.as_deref().map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
            let question = if q.question.trim().is_empty() { q.header.clone() } else { q.question.trim().to_owned() };
            Some((Decision { date: today.to_owned(), question, chosen: chose.join(", "), picture: None, passed, why, ticket: set.ticket }, picture))
        })
        .collect()
}

fn same_question(a: &Decision, b: &Decision) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    a.date == b.date && norm(&a.question) == norm(&b.question)
}

/// Every source's decisions as one record, newest first: the same question
/// on the same day once, the first source's word kept and what it lacks
/// taken from the others.
pub fn merge(sources: Vec<Vec<Decision>>) -> Vec<Decision> {
    let mut out: Vec<Decision> = Vec::new();
    for d in sources.into_iter().flatten() {
        match out.iter_mut().find(|o| same_question(o, &d)) {
            Some(o) => {
                o.picture = o.picture.take().or(d.picture);
                o.why = o.why.take().or(d.why);
                o.ticket = o.ticket.or(d.ticket);
                if o.passed.is_empty() {
                    o.passed = d.passed;
                }
            }
            None => out.push(d),
        }
    }
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out.truncate(MAX);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero() -> Decision {
        Decision {
            date: "2026-11-03".into(),
            question: "히어로 레이아웃".into(),
            chosen: "B — 한 단, 여백 넉넉히".into(),
            picture: Some("decisions/2026-11-03-hero.png".into()),
            passed: vec!["A — 두 단".into(), "C — 꽉 찬 사진".into()],
            why: Some("좀 더 숨 쉬게".into()),
            ticket: Some(272),
        }
    }

    #[test]
    fn a_decision_is_written_and_read_back_as_it_was() {
        let written = text(&hero());
        assert_eq!(
            written,
            "## 2026-11-03 · 히어로 레이아웃\n- Chose: B — 한 단, 여백 넉넉히\n- Picture: decisions/2026-11-03-hero.png\n- Passed over: A — 두 단; C — 꽉 찬 사진\n- Why: \u{201c}좀 더 숨 쉬게\u{201d}\n- Ticket: T-272\n"
        );
        assert_eq!(parse(&written), [hero()]);
        let bare = Decision { date: "2026-11-04".into(), question: "Accent\ncolour".into(), chosen: "Clay".into(), ..Decision::default() };
        assert_eq!(text(&bare), "## 2026-11-04 · Accent colour\n- Chose: Clay\n", "one line each");
    }

    #[test]
    fn many_in_one_file_and_the_old_templates_entries_read_the_same() {
        let file = format!("# Decisions\n\nWhat was chosen.\n\n{}\n## 2026-10-01 · Accent\n- Chose: Clay\n", text(&hero()));
        assert_eq!(parse(&file).len(), 2);
        let old = "# Taste\n\n## YYYY-MM-DD · what was decided\n- Chose: (the option, with its picture's path)\n- In their words: \"…\"\n\n## 2026-10-02 · Logo direction\n- Chose: the dot mark (`directions/b.png`)\n- Passed over: (the others)\n- In their words: \"simple, like a stamp\"\n- What it says: plain\n```md\n## 2026-01-01 · not one\n- Chose: x\n```\n";
        let read = parse(old);
        assert_eq!(read.len(), 1, "{read:?}");
        assert_eq!(read[0], Decision { date: "2026-10-02".into(), question: "Logo direction".into(), chosen: "the dot mark".into(), picture: Some("directions/b.png".into()), why: Some("simple, like a stamp".into()), ..Decision::default() });
    }

    #[test]
    fn what_equipe_learned_in_the_memory_reads_as_decisions() {
        let body = "## 2026-11-03 · T-272 (designer)\n- Which hero layout? \u{2192} chose B: airy, one column (the recommendation); passed over A: dense, C: full; in their words: \u{201c}좀 더 숨 쉬게\u{201d}\n- Accent colour? \u{2192} chose Clay; passed over Blue\n- Anything else?; in their words: \u{201c}로고는 작게\u{201d}\n\n## 2026-11-02 · T-336 (designer)\n- Sent back \u{201c}Pricing\u{201d}: \u{201c}답답해\u{201d}";
        let read = from_history(body);
        assert_eq!(read.len(), 3, "{read:?}");
        assert_eq!(read[0], Decision { date: "2026-11-03".into(), question: "Which hero layout?".into(), chosen: "B: airy, one column".into(), passed: vec!["A: dense".into(), "C: full".into()], why: Some("좀 더 숨 쉬게".into()), ticket: Some(272), picture: None });
        assert_eq!((read[1].chosen.as_str(), read[1].why.as_deref()), ("Clay", None));
        assert_eq!((read[2].question.as_str(), read[2].chosen.as_str()), ("Anything else?", "로고는 작게"));
    }

    #[test]
    fn a_visual_pick_answered_becomes_a_decision_with_its_picture() {
        use crate::protocol::decide::{Answer, Choice, Question, QuestionSet};
        let choice = |label: &str, image: Option<&str>| Choice { label: label.into(), images: image.map(|i| vec![i.to_owned()]).unwrap_or_default(), ..Default::default() };
        let set = QuestionSet {
            id: 1,
            session: 1,
            member: "designer".into(),
            ticket: Some(272),
            questions: vec![
                Question { question: "히어로 레이아웃".into(), options: vec![choice("A — 두 단", Some("/w/a.png")), choice("B — 한 단 (Recommended)", Some("/w/b.png"))], ..Default::default() },
                Question { question: "말투".into(), options: vec![choice("해요체", None), choice("합니다체", None)], ..Default::default() },
                Question { question: "그림 없이 안 고름".into(), options: vec![choice("x", Some("/w/x.png"))], ..Default::default() },
            ],
            asked_at: 0,
            review: false,
            retro: None,
        };
        let answers = vec![Answer { picked: vec![1], other: Some(" 숨 쉬게 ".into()), ..Answer::default() }, Answer { picked: vec![0], other: None, ..Answer::default() }, Answer::default()];
        let found = from_answers(&set, &answers, "2026-11-04");
        assert_eq!(found.len(), 1, "only a pick with a picture: words alone stay in what equipe learned");
        let (d, picture) = &found[0];
        assert_eq!(picture, "/w/b.png");
        assert_eq!(d, &Decision { date: "2026-11-04".into(), question: "히어로 레이아웃".into(), chosen: "B — 한 단".into(), picture: None, passed: vec!["A — 두 단".into()], why: Some("숨 쉬게".into()), ticket: Some(272) });
    }

    #[test]
    fn merged_newest_first_the_same_question_once_with_the_most_said() {
        let mut from_file = hero();
        from_file.why = None;
        let learned = Decision { question: "히어로  레이아웃".into(), chosen: "B: airy".into(), why: Some("숨 쉬게".into()), picture: None, ..hero() };
        let older = Decision { date: "2026-10-01".into(), question: "Accent".into(), chosen: "Clay".into(), ..Decision::default() };
        let newer = Decision { date: "2026-11-04".into(), question: "Logo".into(), chosen: "Dot".into(), ..Decision::default() };
        let merged = merge(vec![vec![older.clone(), from_file], vec![learned, newer.clone()]]);
        assert_eq!(merged.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(), ["2026-11-04", "2026-11-03", "2026-10-01"]);
        assert_eq!(merged[1].chosen, "B — 한 단, 여백 넉넉히", "the file's word kept");
        assert_eq!(merged[1].why.as_deref(), Some("숨 쉬게"), "what it lacked, taken");
        assert_eq!(merged[1].picture.as_deref(), Some("decisions/2026-11-03-hero.png"));
        let many: Vec<Decision> = (0..MAX + 5).map(|i| Decision { date: format!("2026-01-{:02}", i % 28 + 1), question: format!("q{i}"), chosen: "x".into(), ..Decision::default() }).collect();
        assert_eq!(merge(vec![many]).len(), MAX);
    }
}
