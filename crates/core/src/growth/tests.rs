use serde_json::json;

use super::*;

const NOW: u64 = 1_800_000_000;

fn ev(id: &str, metric: &str, value: f64) -> Evidence {
    Evidence {
        id: id.into(),
        metric: metric.into(),
        label: metric.into(),
        value,
        previous: Some(value * 1.2),
        unit: "%".into(),
        from: "2026-11-09".into(),
        to: "2026-11-15".into(),
        query: "SELECT 1".into(),
        source: "posthog m1".into(),
        better: Better::Up,
        measure: None,
    }
}

fn store() -> Store {
    Store { evidence: vec![ev("E1", "funnel:question_sent>answer_received", 41.2), ev("E2", "event:signup", 120.0)], last_run: Some(NOW - DAY), ..Store::default() }
}

fn card_args(extra: serde_json::Value) -> serde_json::Value {
    let mut c = json!({
        "title": "Show an answer preview while it streams",
        "change": "Stream the first sentence of the answer within a second.",
        "evidence": ["E1"],
        "metric": "funnel:question_sent>answer_received",
        "expected": "+5 points conversion",
        "size": "M",
        "risk": "low",
    });
    for (k, v) in extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    json!({ "cards": [c] })
}

#[test]
fn a_card_stands_on_the_runs_numbers_or_isnt_made() {
    let s = store();
    let made = cards(&card_args(json!({})), &s, &Config::default(), &[], NOW).unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].id, 1);
    assert_eq!(made[0].evidence[0].value, 41.2, "the numbers travel with the card");
    assert_eq!(made[0].status, Status::Open);

    let no_evidence = cards(&card_args(json!({ "evidence": [] })), &s, &Config::default(), &[], NOW).unwrap_err();
    assert!(no_evidence.contains("without numbers it isn't made"), "{no_evidence}");
    let unknown = cards(&card_args(json!({ "evidence": ["E9"] })), &s, &Config::default(), &[], NOW).unwrap_err();
    assert!(unknown.contains("no evidence E9"), "{unknown}");
    let wrong_metric = cards(&card_args(json!({ "metric": "event:other" })), &s, &Config::default(), &[], NOW).unwrap_err();
    assert!(wrong_metric.contains("none of its evidence's"), "{wrong_metric}");
    let mut nan = store();
    nan.evidence[0].value = f64::NAN;
    assert!(cards(&card_args(json!({})), &nan, &Config::default(), &[], NOW).is_err(), "a number that isn't one is no number");

    let stale = Store { last_run: Some(NOW - EVIDENCE_FRESH - 1), ..store() };
    assert!(cards(&card_args(json!({})), &stale, &Config::default(), &[], NOW).unwrap_err().contains("no fresh numbers"));
    assert!(cards(&card_args(json!({})), &Store::default(), &Config::default(), &[], NOW).is_err());
}

#[test]
fn a_card_says_its_size_risk_and_effect_and_names_no_one() {
    let s = store();
    for (extra, said) in [
        (json!({ "size": "huge" }), "a `size`"),
        (json!({ "risk": "none" }), "a `risk`"),
        (json!({ "expected": "" }), "`expected`"),
        (json!({ "title": "" }), "`title`"),
        (json!({ "change": "Email jane.doe@example.com a coupon" }), "an email address"),
        (json!({ "change": "Call +1 415 555 0100 back" }), "a phone number"),
        (json!({ "project": "elsewhere" }), "no project"),
    ] {
        let e = cards(&card_args(extra.clone()), &s, &Config::default(), &["site".into()], NOW).unwrap_err();
        assert!(e.contains(said), "{extra}: {e}");
    }
    let allowed = Config { allowed_fields: vec!["$email".into()], ..Config::default() };
    assert!(cards(&card_args(json!({ "change": "Email jane.doe@example.com a coupon" })), &s, &allowed, &[], NOW).is_ok(), "a field the project allows");
    assert!(cards(&card_args(json!({ "change": "Call +1 415 555 0100 back" })), &s, &allowed, &[], NOW).is_err(), "only that field");
}

#[test]
fn open_cards_are_capped_and_a_no_is_remembered() {
    let mut s = store();
    let config = Config { max_cards: Some(2), ..Config::default() };
    let first = cards(&card_args(json!({})), &s, &config, &[], NOW).unwrap();
    s.cards.extend(first.clone());
    s.next = 1;
    let again = cards(&card_args(json!({})), &s, &config, &[], NOW).unwrap_err();
    assert!(again.contains("already open"), "{again}");
    let other = card_args(json!({ "title": "Ask for signup after the first answer", "evidence": ["E2"], "metric": "event:signup" }));
    let second = cards(&other, &s, &config, &[], NOW).unwrap();
    assert_eq!(second[0].id, 2);
    s.cards.extend(second);
    s.next = 2;
    let full = cards(&card_args(json!({ "title": "Something else entirely here" })), &s, &config, &[], NOW).unwrap_err();
    assert!(full.contains("only 0 more may be open"), "{full}");

    // The person says no to G-1: not proposed again for a while, even worded a little differently.
    let declined = s.cards[0].clone();
    s.cards[0].status = Status::Declined;
    decline(&mut s, &declined, NOW);
    let reworded = card_args(json!({ "title": "Show the answer preview while streaming" }));
    let e = cards(&reworded, &s, &config, &[], NOW).unwrap_err();
    assert!(e.contains("said no to G-1"), "{e}");
    let later = Store { last_run: Some(NOW + DECLINED_FOR), ..s.clone() };
    assert_eq!(cards(&reworded, &later, &config, &[], NOW + DECLINED_FOR + 1).map(|c| c.len()), Ok(1), "after a while it may come back");
}

#[test]
fn the_answer_on_a_card_is_accept_later_or_no() {
    let pick = |n: usize| Answer { picked: vec![n], ..Answer::default() };
    assert_eq!(answered(Some(&pick(0))), Status::Accepted);
    assert_eq!(answered(Some(&pick(1))), Status::Later);
    assert_eq!(answered(Some(&pick(2))), Status::Declined);
    assert_eq!(answered(Some(&Answer::default())), Status::Later, "skipped: later");
    assert_eq!(answered(None), Status::Later);
    let q = question(&cards(&card_args(json!({})), &store(), &Config::default(), &[], NOW).unwrap()[0], Lang::En);
    assert_eq!(q.options.len(), 3);
    assert!(q.question.contains("41.2%") && q.question.contains("2026-11-09"), "the numbers are on the card: {}", q.question);
}

#[test]
fn traffic_is_left_out_as_the_project_says_and_equipes_own_by_default() {
    let default = exclusion(&Config::default()).unwrap();
    assert!(default.starts_with(" AND NOT ("));
    assert!(default.contains("coalesce(toString(properties.internal), '') = 'true'"));
    assert!(default.contains("coalesce(toString(properties.equipe_agent), '') = 'true'"));
    assert!(default.contains("positionCaseInsensitive(coalesce(toString(properties.$raw_user_agent), ''), 'equipe') > 0"));
    let config = Config { exclude: vec!["$host=localhost".into(), "email~@test.example".into(), "environment!=production".into()], exclude_equipe: false, ..Config::default() };
    let own = exclusion(&config).unwrap();
    assert!(!own.contains("equipe_agent"), "turned off: {own}");
    assert!(own.contains("coalesce(toString(properties.$host), '') = 'localhost'"));
    assert!(own.contains("positionCaseInsensitive(coalesce(toString(properties.email), ''), '@test.example') > 0"));
    assert!(own.contains("coalesce(toString(properties.environment), '') != 'production'"));
    assert_eq!(exclusion(&Config { exclude_equipe: false, ..Config::default() }).unwrap(), "");
    for bad in ["internal", "a b=1", "x) OR (1=1"] {
        assert!(exclusion(&Config { exclude: vec![bad.into()], ..Config::default() }).is_err(), "{bad}");
    }
    assert_eq!(condition("note=it's").unwrap(), "coalesce(toString(properties.note), '') = 'it\\'s'");

    // Every query of a run carries it, and is a read-only SELECT.
    let config: Config = serde_json::from_value(json!({
        "keyEvents": ["question_sent"],
        "funnels": ["question_sent → answer_received → answer_rated", { "steps": ["signup", "question_sent"], "windowDays": 3 }],
        "audiences": [{ "name": "ko", "where": "$browser_language~ko" }],
        "exclude": ["$host=localhost"],
    }))
    .unwrap();
    let all = asks(&config, "m1").unwrap();
    assert_eq!(all.iter().map(|a| a.metric.as_str()).collect::<Vec<_>>(), [
        "event:question_sent",
        "funnel:question_sent>answer_received",
        "funnel:answer_received>answer_rated",
        "funnel:question_sent>answer_rated",
        "funnel:question_sent>answer_rated@ko",
        "funnel:signup>question_sent",
    ]);
    for a in &all {
        let q = measure_query(&a.measure, 100, 106).unwrap();
        assert!(q.contains("'localhost'") && q.contains("equipe_agent"), "{q}");
        crate::metrics::hogql::select_only(&q).unwrap_or_else(|e| panic!("{e}: {q}"));
    }
    assert!(measure_query(&all[4].measure, 100, 106).unwrap().contains("positionCaseInsensitive(coalesce(toString(properties.$browser_language), ''), 'ko')"));
}

#[test]
fn a_follow_up_is_worked_no_change_or_worse() {
    assert_eq!(classify(40.0, 44.0, 200.0, Better::Up), (Verdict::Worked, Some(10.0)));
    assert_eq!(classify(40.0, 41.0, 200.0, Better::Up).0, Verdict::NoChange, "under 5%");
    assert_eq!(classify(40.0, 30.0, 200.0, Better::Up).0, Verdict::Worse);
    assert_eq!(classify(100.0, 80.0, 200.0, Better::Down).0, Verdict::Worked, "fewer errors");
    assert_eq!(classify(100.0, 130.0, 200.0, Better::Down).0, Verdict::Worse);
    assert_eq!(classify(40.0, 80.0, 5.0, Better::Up).0, Verdict::NoChange, "too few to say");
    assert_eq!(classify(0.0, 12.0, 30.0, Better::Up), (Verdict::Worked, None));
    assert_eq!(classify(0.0, 0.0, 30.0, Better::Up), (Verdict::NoChange, None));
}

#[test]
fn the_loop_keeps_to_its_weekly_budget() {
    let mut s = Store::default();
    assert!(may_spend(&s, 100_000, 30_000, NOW).is_ok());
    s.runs.push(Run { at: NOW - DAY, tokens: 80_000, evidence: 3, by: "weekly".into() });
    let e = may_spend(&s, 100_000, 30_000, NOW).unwrap_err();
    assert!(e.contains("80000 of its 100000"), "{e}");
    assert!(may_spend(&s, 100_000, 30_000, NOW - DAY + WEEK).is_ok(), "a week on, room again");
    assert_eq!(estimate(&"x".repeat(4_000)), 1_000 + TURN_ALLOWANCE);
    assert_eq!(Config::default().weekly_tokens(), WEEKLY_TOKENS);
}

#[test]
fn a_card_live_gets_its_follow_ups_and_worse_brings_an_undo() {
    let mut card = cards(&card_args(json!({})), &store(), &Config::default(), &[], NOW).unwrap().remove(0);
    card.status = Status::Accepted;
    card.ticket = Some(12);
    let ticket = Ticket { id: 12, project: Some("/p".into()), status: TicketStatus::Done, ..ticket() };
    assert_eq!(live_at(&ticket, &[], NOW), None, "merged isn't live: nothing deployed it");
    let deploy = |at: u64, tickets: Vec<u32>| Deploy { id: 1, project: "site".into(), path: "/p".into(), source: Default::default(), environment: None, url: None, from: None, to: "abc".into(), at, last_at: at, items: vec![], internal: 0, tickets, told: true };
    assert_eq!(live_at(&ticket, &[deploy(NOW + 50, vec![12]), deploy(NOW + 10, vec![3, 12])], NOW + 60), Some(NOW + 10));
    let home_task = Ticket { project: None, ..ticket.clone() };
    assert_eq!(live_at(&home_task, &[], NOW + 99), Some(NOW + 99), "work in the squad's folder is live once done");

    went_live(&mut card, NOW + 10);
    assert_eq!(card.status, Status::Live);
    assert_eq!(card.follow_ups.iter().map(|f| f.due_at).collect::<Vec<_>>(), [NOW + 10 + 7 * DAY, NOW + 10 + 14 * DAY]);
    assert!(due(&card, NOW + 6 * DAY).is_empty());
    assert_eq!(due(&card, NOW + 10 + 7 * DAY), [7]);
    let ((bf, bt), (af, at)) = windows(NOW + 10, 7);
    assert_eq!((bt - bf + 1, at - af + 1, af - bt), (7, 7, 1), "seven days before, seven after, back to back");

    let outcome = Outcome { verdict: Verdict::Worse, before: 41.2, after: 30.0, change_pct: Some(-27.2), before_from: "a".into(), before_to: "b".into(), after_from: "c".into(), after_to: "d".into(), query: "SELECT 2".into(), at: NOW, failed: None };
    assert!(!measured(&mut card, 7, outcome.clone()), "one of two");
    assert!(outcome_line(&card, 7, &outcome).contains("worse"));
    let undo = revert_card(&card, &outcome, 9, NOW);
    assert_eq!(undo.reverts, Some(card.id));
    assert_eq!(undo.evidence[0].value, 30.0, "the undo stands on the follow-up's numbers");
    assert!(undo.title.starts_with("Undo G-1"));
    assert!(measured(&mut card, 14, outcome));
    assert_eq!(card.status, Status::Measured);
}

#[test]
fn the_weekly_run_is_mondays_once() {
    use crate::routines::Utc;
    // 2026-11-16 is a Monday.
    let monday_10 = (crate::metrics::day_number("2026-11-16").unwrap() as u64) * DAY + 10 * 3600;
    assert!(weekly_due(None, monday_10, &Utc));
    assert!(!weekly_due(None, monday_10 - 2 * 3600, &Utc), "before 09:00");
    assert!(!weekly_due(Some(monday_10 - 3600), monday_10, &Utc), "ran this morning");
    assert!(!weekly_due(None, monday_10 + DAY, &Utc), "Tuesday");
    assert!(weekly_due(Some(monday_10 - WEEK), monday_10, &Utc));
}

#[test]
fn funnels_read_as_text_or_steps() {
    let c: Config = serde_json::from_value(json!({ "funnels": ["question_sent → answer_received", "a -> b > c"] })).unwrap();
    assert_eq!(c.funnels[0].steps, ["question_sent", "answer_received"]);
    assert_eq!(c.funnels[1].steps, ["a", "b", "c"]);
    assert!(c.exclude_equipe, "equipe's own traffic is left out unless said");
    assert_eq!(c.cards(), CARDS);
    let rows = vec![vec![json!(200), json!(50)]];
    let m = Measure::Funnel { connection: "m1".into(), from: "a".into(), to: "b".into(), days: 1, exclude: String::new(), audience: String::new() };
    assert_eq!(read_value(&m, &rows), Some((25.0, 200.0)));
    assert_eq!(read_value(&Measure::Event { connection: "m1".into(), event: "a".into(), exclude: String::new() }, &[vec![json!("17")]]), Some((17.0, 17.0)));
}

fn ticket() -> Ticket {
    serde_json::from_value(json!({ "id": 1, "title": "t", "body": "", "status": "todo", "kind": "code", "depends_on": [], "created_by": "pm" })).unwrap()
}
