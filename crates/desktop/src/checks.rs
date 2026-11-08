//! A design check and a second look as a member's conversation shows them:
//! read from the summary line `equipe design lint` ends with and the verdict
//! line `equipe design second-look` ends with — only what ran and what it
//! printed, never guessed. The words here; the cards in `views/chat.rs`.

use equipe_core::design::lint::record::{Fix, Verdict};
use equipe_core::design::lint::{self, Summary};
use equipe_core::design::second_look;
use equipe_core::t;

/// What a call ran, when it was a design check or a second look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    Check(Box<Summary>),
    Look { screen: String, verdict: Box<Verdict> },
}

/// The design check or second look a finished command was, from what it printed.
pub fn ran(tool: &str, target: &str, output: &str) -> Option<Ran> {
    if tool != "Bash" {
        return None;
    }
    if target.contains("design second-look") {
        return second_look::in_output(output).map(|(screen, verdict)| Ran::Look { screen, verdict: Box::new(verdict) });
    }
    if target.contains("design lint") && !target.contains("--hook") {
        return lint::summary_in(output).map(|s| Ran::Check(Box::new(s)));
    }
    None
}

/// A rule's name in the person's language; its id when it has none.
pub fn rule_name(id: &str) -> String {
    let key = format!("check.rule.{id}");
    let word = crate::i18n::catalog().get(&key);
    if word == key { id.to_owned() } else { word.to_owned() }
}

/// The floor's count: `Floor 11/11`.
pub fn floor(s: &Summary) -> String {
    t!("check.floor", passed = s.floor.0, total = s.floor.1)
}

/// What else it found, in order: habits to fix, habits kept, finishing notes.
pub fn counts(s: &Summary) -> Vec<String> {
    let mut out = Vec::new();
    if s.habits > 0 {
        out.push(t!("check.habits", count = s.habits));
    }
    if !s.waived.is_empty() {
        out.push(t!("check.kept", count = s.waived.len()));
    }
    if s.finish > 0 {
        out.push(t!("check.notes", count = s.finish));
    }
    out
}

/// Where it was drawn: each width, or that only its source was read.
pub fn widths(s: &Summary) -> String {
    if s.widths.is_empty() { t!("check.source_only").to_owned() } else { s.widths.iter().map(u32::to_string).collect::<Vec<_>>().join(" · ") }
}

/// One line of the details: what kind it is, the rule, what was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub kind: Kind,
    pub rule: String,
    pub said: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The floor failed here.
    Fails,
    /// A habit to fix or keep with a reason.
    Habit,
    /// A habit kept: `said` is the reason.
    Kept,
    /// A finishing note.
    Note,
    /// A rule that couldn't run.
    Skipped,
}

pub fn details(s: &Summary) -> Vec<Detail> {
    let mut out = Vec::new();
    let mut add = |kind: Kind, list: &[(String, String)]| {
        for (id, said) in list {
            out.push(Detail { kind, rule: rule_name(id), said: said.clone() });
        }
    };
    add(Kind::Fails, &s.failing);
    add(Kind::Habit, &s.open);
    add(Kind::Kept, &s.waived);
    add(Kind::Note, &s.notes);
    for id in &s.skipped {
        out.push(Detail { kind: Kind::Skipped, rule: rule_name(id), said: String::new() });
    }
    out
}

/// What a second look said, in a line: it passed, or how many fixes it asks for.
pub fn verdict(v: &Verdict) -> String {
    if v.pass { t!("look.pass").to_owned() } else { t!("look.fixes", count = v.fixes.len()) }
}

/// A fix as one line: where, what, and why when it says.
pub fn fix(f: &Fix) -> String {
    if f.why.is_empty() { format!("{} — {}", f.place, f.what) } else { format!("{} — {} ({})", f.place, f.what, f.why) }
}

/// Who looked, in a line: fresh eyes, and what they were given.
pub fn who(v: &Verdict) -> String {
    let saw = if v.first { t!("look.saw_first") } else { t!("look.saw") };
    if v.by == "subagent" { format!("{saw} · {}", t!("look.by_subagent")) } else { saw.to_owned() }
}

/// Where a screen's latest check leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The floor fails.
    Fails,
    /// The floor passes; the second look is still to come, or asked for fixes.
    Waiting,
    /// The floor passes and fresh eyes passed it.
    Done,
}

pub fn screen_state(c: &equipe_core::protocol::design::ScreenCheck) -> State {
    match &c.look {
        _ if c.floor.0 < c.floor.1 => State::Fails,
        Some(l) if l.pass => State::Done,
        _ => State::Waiting,
    }
}

/// A screen's check on the UI map, in a line: the floor, then the second look.
pub fn screen_line(c: &equipe_core::protocol::design::ScreenCheck) -> String {
    let floor = t!("check.floor", passed = c.floor.0, total = c.floor.1);
    // A floor that fails is all there is to say: no look makes up for it.
    if c.floor.0 < c.floor.1 {
        return floor;
    }
    let look = match &c.look {
        Some(l) if l.pass => t!("flows.look_pass").to_owned(),
        Some(l) => format!("{} · {}", t!("look.name"), t!("look.fixes", count = l.fixes)),
        None => t!("flows.look_none").to_owned(),
    };
    format!("{floor} · {look}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> Summary {
        Summary {
            floor: (10, 11),
            habits: 1,
            waived: vec![("tracked-caps".into(), "the brand sets labels in capitals".into())],
            finish: 2,
            widths: vec![360, 1280],
            failing: vec![("contrast".into(), "p.hint is 2.3:1".into())],
            open: vec![("nested-cards".into(), "a card inside a card".into())],
            notes: vec![("touch-target".into(), "a 24×24 icon".into()), ("line-length".into(), "main p".into())],
            screen: Some("settings/notifications".into()),
            record: None,
            skipped: vec!["primary-in-view".into()],
            system: Vec::new(),
        }
    }

    #[test]
    fn a_check_is_read_only_from_what_lint_printed_last() {
        let s = summary();
        let out = format!("Drawing…\n{}\n", lint::mark_line(&s));
        assert_eq!(ran("Bash", "equipe design lint design/mockups/a.html --render", &out), Some(Ran::Check(Box::new(s.clone()))));
        assert_eq!(ran("Bash", "equipe design lint --hook", &out), None, "the edit hook isn't a check the person sees");
        assert_eq!(ran("Bash", "cargo test", &out), None);
        assert_eq!(ran("Read", "equipe design lint", &out), None);
        assert_eq!(ran("Bash", "equipe design lint a.html", "it stopped"), None, "nothing shown that didn't run to its end");
        let v = Verdict { pass: false, fixes: vec![Fix { place: "form at 360".into(), what: "stack the buttons".into(), why: String::new() }], ..Verdict::default() };
        let out = second_look::mark_line("settings/notifications", &v);
        assert_eq!(ran("Bash", "equipe design second-look settings/notifications", &out), Some(Ran::Look { screen: "settings/notifications".into(), verdict: Box::new(v) }));
    }

    #[test]
    fn its_words_count_what_it_found_and_name_each_rule() {
        let s = summary();
        assert_eq!(floor(&s), "기준선 10/11");
        assert_eq!(counts(&s), vec!["고칠 습관 1개", "유지 1개", "다듬을 점 2개"]);
        assert_eq!(widths(&s), "360 · 1280");
        assert_eq!(widths(&Summary::default()), "소스만 읽음");
        let d = details(&s);
        assert_eq!(d.iter().map(|d| d.kind).collect::<Vec<_>>(), vec![Kind::Fails, Kind::Habit, Kind::Kept, Kind::Note, Kind::Note, Kind::Skipped]);
        assert_eq!(d[0].rule, "대비");
        assert_eq!(d[2].said, "the brand sets labels in capitals");
        assert_eq!(rule_name("not-a-rule"), "not-a-rule");
        let v = Verdict { pass: false, fixes: vec![Fix { place: "form".into(), what: "stack".into(), why: "cut off".into() }; 2], ..Verdict::default() };
        assert_eq!(verdict(&v), "고칠 점 2개");
        assert_eq!(fix(&v.fixes[0]), "form — stack (cut off)");
        assert_eq!(verdict(&Verdict { pass: true, ..Verdict::default() }), "통과");
    }

    #[test]
    fn a_screen_on_the_map_says_its_floor_and_its_second_look() {
        use equipe_core::protocol::design::{Look, ScreenCheck};
        let done = ScreenCheck { floor: (11, 11), look: Some(Look { pass: true, fixes: 0 }), ..ScreenCheck::default() };
        assert_eq!(screen_state(&done), State::Done);
        assert_eq!(screen_line(&done), "기준선 11/11 · 새 시선 통과");
        let asked = ScreenCheck { floor: (11, 11), look: Some(Look { pass: false, fixes: 2 }), ..ScreenCheck::default() };
        assert_eq!(screen_state(&asked), State::Waiting);
        assert_eq!(screen_line(&asked), "기준선 11/11 · 새 시선 · 고칠 점 2개");
        let failing = ScreenCheck { floor: (9, 11), look: Some(Look { pass: true, fixes: 0 }), ..ScreenCheck::default() };
        assert_eq!(screen_state(&failing), State::Fails, "a pass doesn't cover a floor that fails");
        assert_eq!(screen_line(&failing), "기준선 9/11");
        assert_eq!(screen_line(&ScreenCheck { floor: (10, 10), ..ScreenCheck::default() }), "기준선 10/10 · 새 시선 아직 없음");
    }

    #[test]
    fn every_rule_has_a_name_in_every_language() {
        for lang in equipe_core::i18n::Lang::ALL {
            let words = crate::i18n::of(lang);
            for r in lint::RULES {
                assert!(words.own(&format!("check.rule.{}", r.id)).is_some(), "{lang:?}: check.rule.{}", r.id);
            }
        }
    }
}
