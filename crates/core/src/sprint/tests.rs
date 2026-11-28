use super::*;
use crate::routines::Utc;
use equipe_protocol::decide::Answer;
use equipe_protocol::sprint::{Arm, Judged, Split};
use serde_json::json;

/// Friday 2026-11-06 17:00 UTC.
const FRI_5: u64 = 1_793_984_400;

fn evidence(id: &str, metric: &str, measure: Option<Measure>) -> Evidence {
    Evidence {
        id: id.into(),
        metric: metric.into(),
        label: format!("{metric} label"),
        value: 18.2,
        previous: Some(17.0),
        unit: "%".into(),
        from: "2026-11-01".into(),
        to: "2026-11-07".into(),
        query: "SELECT …".into(),
        source: "posthog m1".into(),
        better: Better::Up,
        measure,
    }
}

fn store() -> Store {
    let funnel = Measure::Funnel { connection: "m1".into(), from: "visit".into(), to: "signup".into(), days: 3, exclude: String::new(), audience: String::new() };
    Store { evidence: vec![evidence("E1", "funnel:visit>signup", Some(funnel)), evidence("E2", "health:api", None)], ..Store::default() }
}

fn item_json(title: &str, impact: &str, size: &str, risk: &str) -> Value {
    json!({ "kind": "iterate", "title": title, "change": "Do the thing.", "why": "E1 dropped", "expected": "+10% sign-ups", "impact": impact, "size": size, "risk": risk })
}

fn plan_args(items: Vec<Value>) -> Value {
    json!({ "north_star": { "name": "people who sign up each week", "metric": "event:signup" }, "goal": "More sign-ups", "done_when": "The short form is live", "items": items })
}

fn ctx<'a>(store: &'a Store, team: &'a [String], projects: &'a [String]) -> PlanCtx<'a> {
    PlanCtx { store, projects, team, reviewing: None, now: FRI_5 }
}

fn team() -> Vec<String> {
    ["pm", "pm", "frontend", "frontend", "data", "data"].map(String::from).to_vec()
}

#[test]
fn a_sprint_runs_a_week_to_the_review_slot_after_half_its_length() {
    let s = Settings::default();
    assert_eq!((length(&s), budget(&s), slot(&s, None)), (7, BUDGET, (5, 17)));
    assert_eq!(slot(&s, Some((4, 10))), (4, 10), "the retro's slot, when it was moved");
    assert_eq!(slot(&Settings { weekday: Some(1), hour: Some(9), ..s.clone() }, Some((4, 10))), (1, 9));
    assert_eq!(length(&Settings { length_days: 14, ..s.clone() }), 14);
    // Started on Saturday: this Friday. Started on Thursday: next Friday, not tomorrow.
    let saturday = FRI_5 + DAY;
    assert_eq!(ends_at(saturday, (5, 17), 7, &Utc), FRI_5 + 7 * DAY);
    let thursday = FRI_5 - DAY;
    assert_eq!(ends_at(thursday, (5, 17), 7, &Utc), FRI_5 + 7 * DAY);
    // A fortnight: the slot of its second week.
    assert_eq!(ends_at(saturday, (5, 17), 14, &Utc), FRI_5 + 14 * DAY);
}

#[test]
fn the_loop_runs_once_a_product_is_live_unless_the_person_turns_it_off() {
    let auto = Settings::default();
    assert!(enabled(&auto, true, true).is_ok());
    assert!(enabled(&auto, true, false).unwrap_err().contains("nothing is live"));
    assert!(enabled(&auto, false, true).unwrap_err().contains("no PM"));
    assert!(enabled(&Settings { on: Some(false), ..auto.clone() }, true, true).unwrap_err().contains("turned the sprint loop off"));
    assert!(enabled(&Settings { on: Some(true), ..auto }, true, false).is_ok(), "on by the person before anything is live");
}

#[test]
fn metrics_are_read_from_shorthand_an_evidence_or_an_object() {
    let s = store();
    assert_eq!(metric_of(&json!("event:signup"), &[]).unwrap(), Metric::Event { event: "signup".into(), filter: String::new() });
    assert_eq!(metric_of(&json!("signup"), &[]).unwrap(), Metric::Event { event: "signup".into(), filter: String::new() });
    assert_eq!(metric_of(&json!("visit > signup"), &[]).unwrap(), Metric::Conversion { from: "visit".into(), to: "signup".into(), days: 1, filter: String::new() });
    // An evidence's metric keeps its days.
    assert_eq!(metric_of(&json!("funnel:visit>signup"), &s.evidence).unwrap(), Metric::Conversion { from: "visit".into(), to: "signup".into(), days: 3, filter: String::new() });
    assert!(metric_of(&json!("health:api"), &s.evidence).unwrap_err().contains("can't be read again"));
    let o = json!({ "kind": "conversion", "from": "visit", "to": "signup", "days": 2, "filter": "utm_source=newsletter" });
    assert_eq!(metric_of(&o, &[]).unwrap(), Metric::Conversion { from: "visit".into(), to: "signup".into(), days: 2, filter: "utm_source=newsletter".into() });
    assert!(metric_of(&json!({ "kind": "event", "event": "x", "filter": "no operator" }), &[]).is_err());
    assert!(metric_of(&json!({ "kind": "conversion", "from": "a", "to": "b", "days": 90 }), &[]).is_err());
}

#[test]
fn a_rule_is_a_change_some_days_and_the_fewest_counted() {
    let r = rule_of(Some(&json!({ "min_change_pct": 10, "window_days": 7, "min_sample": 200 }))).unwrap();
    assert_eq!((r.min_change_pct, r.window_days, r.min_sample, r.better), (10.0, 7, 200, Better::Up));
    assert_eq!(rule_of(Some(&json!({ "min_change_pct": 5, "window_days": 7, "min_sample": 1, "better": "down" }))).unwrap().better, Better::Down);
    for bad in [json!({ "min_change_pct": 0, "window_days": 7, "min_sample": 1 }), json!({ "min_change_pct": 5, "window_days": 31, "min_sample": 1 }), json!({ "min_change_pct": 5, "window_days": 7, "min_sample": 0 }), json!({})] {
        assert!(rule_of(Some(&bad)).is_err(), "{bad}");
    }
    let conversion = Metric::Conversion { from: "a".into(), to: "b".into(), days: 1, filter: String::new() };
    assert_eq!(split_of(Some(&json!({ "flag": "short-form" })), &conversion).unwrap(), Some(Split { flag: "short-form".into(), control: "control".into() }));
    assert!(split_of(Some(&json!({ "flag": "x" })), &Metric::Event { event: "a".into(), filter: String::new() }).unwrap_err().contains("share of people"));
    assert!(split_of(Some(&json!({ "flag": "has space" })), &conversion).is_err());
}

#[test]
fn a_plan_needs_a_goal_done_the_north_star_once_and_grounded_items() {
    let s = store();
    let (t, p) = (team(), vec!["site".to_owned()]);
    let made = plan(&plan_args(vec![item_json("Shorter signup form", "high", "S", "low")]), &ctx(&s, &t, &p)).unwrap();
    assert_eq!((made.goal.as_str(), made.items.len(), made.items[0].id, made.items[0].sprint), ("More sign-ups", 1, 1, 1));
    assert_eq!(made.north_star.unwrap().metric, Some(Metric::Event { event: "signup".into(), filter: String::new() }));
    assert_eq!(made.items[0].cost_tokens, cost(Size::S));

    // Everything wrong is said at once.
    let bad = json!({ "items": [
        { "kind": "teleport", "title": "", "change": "x", "expected": "y", "impact": "huge", "size": "XL", "risk": "low" },
        { "kind": "fix", "title": "Mail a@b.co", "change": "x", "why": "y", "expected": "z", "impact": "low", "size": "S", "risk": "low", "evidence": ["E9"], "owner": "mobile", "project": "api" },
    ]});
    let why = plan(&bad, &ctx(&s, &t, &p)).unwrap_err();
    for must in ["a `goal`", "`done_when`", "the `north_star`", "item 1: a `title`", "item 1: `why`", "item 1: a `kind`", "item 1: an `impact`", "item 1: a `size`", "item 2: it names an email address", "item 2: there is no E9", "item 2: there is no mobile on the team", "item 2: the squad has no project \"api\""] {
        assert!(why.contains(must), "{must}\n{why}");
    }
    // Six items: five at most.
    let six = plan(&plan_args((0..6).map(|n| item_json(&format!("Idea number {n} about {}", ["a", "b", "c", "d", "e", "f"][n]), "low", "S", "low")).collect()), &ctx(&s, &t, &p)).unwrap_err();
    assert!(six.contains("at most 5"), "{six}");
    // The same item twice.
    let twice = plan(&plan_args(vec![item_json("Shorter signup form", "high", "S", "low"), item_json("Shorter signup form now", "low", "S", "low")]), &ctx(&s, &t, &p)).unwrap_err();
    assert!(twice.contains("the same item"), "{twice}");
}

#[test]
fn items_cite_the_reviews_numbers_and_results_and_carry_a_hypothesis() {
    let mut s = store();
    s.north_star = Some(NorthStar { name: "x".into(), metric: None, better: Better::Up, value: None, previous: None });
    let h = Hypothesis { belief: "b".into(), metric: Metric::Event { event: "signup".into(), filter: String::new() }, rule: rule_of(Some(&json!({ "min_change_pct": 10, "window_days": 7, "min_sample": 10 }))).unwrap(), split: None, events: vec![] };
    let mut x = experiment(3, 1, &h, Some(12), None, "pm", FRI_5);
    x.status = ExperimentStatus::Judged;
    s.experiments.push(x);
    let mut it = item_json("Do more of the short form", "medium", "M", "medium");
    it["evidence"] = json!(["E1", "H-3"]);
    it["owner"] = json!("frontend");
    it["hypothesis"] = json!({ "belief": "A shorter form gets more sign-ups", "metric": "funnel:visit>signup", "rule": { "min_change_pct": 10, "window_days": 7, "min_sample": 200 }, "split": { "flag": "short-form" }, "events": ["signup_started"] });
    let args = json!({ "goal": "g", "done_when": "d", "items": [it] });
    let (t, p) = (team(), vec![]);
    let made = plan(&args, &ctx(&s, &t, &p)).unwrap();
    let item = &made.items[0];
    assert_eq!(item.evidence.len(), 2);
    assert!(item.evidence[0].starts_with("E1 · ") && item.evidence[1].starts_with("H-3 "), "{:?}", item.evidence);
    let h = item.hypothesis.as_ref().unwrap();
    assert_eq!((h.metric.is_share(), h.split.as_ref().map(|s| s.flag.as_str()), h.events.as_slice()), (true, Some("short-form"), &["signup_started".to_owned()][..]));
    assert_eq!(item.owner.as_deref(), Some("frontend"));
}

#[test]
fn a_review_needs_its_retro_and_a_running_or_waiting_sprint_refuses_a_new_plan() {
    let mut s = store();
    s.north_star = Some(NorthStar { name: "x".into(), metric: None, better: Better::Up, value: None, previous: None });
    let (t, p) = (team(), vec![]);
    let args = json!({ "goal": "g", "done_when": "d", "items": [item_json("One", "low", "S", "low")] });
    let reviewing = PlanCtx { reviewing: Some(1), ..ctx(&s, &t, &p) };
    assert!(plan(&args, &reviewing).unwrap_err().contains("a `retro`"));
    let mut with = args.clone();
    with["retro"] = json!({ "shipped": ["The shorter form"], "learned": "- Fewer fields, more sign-ups\n- People want Google sign-in", "wrong": [], "next": ["Add Google sign-in"] });
    let made = plan(&with, &reviewing).unwrap();
    assert_eq!(made.retro.unwrap().learned, ["Fewer fields, more sign-ups", "People want Google sign-in"]);
    let mut long = args.clone();
    long["retro"] = json!({ "shipped": ["a", "b", "c", "d"] });
    assert!(plan(&long, &reviewing).unwrap_err().contains("at most 3 lines"));

    let sprint = |phase| Sprint { id: 2, goal: "g".into(), done_when: "d".into(), phase, proposed_at: 0, started_at: None, ends_at: None, closed_at: None, items: vec![], tickets: vec![], experiments: vec![], budget_tokens: 1, spent_tokens: 0, set: 0, retro: None, facts: None };
    s.sprints = vec![sprint(Phase::Proposed)];
    assert!(plan(&args, &ctx(&s, &t, &p)).unwrap_err().contains("S-2 waits on the person"));
    s.sprints = vec![sprint(Phase::Active)];
    assert!(plan(&args, &ctx(&s, &t, &p)).unwrap_err().contains("S-2 runs until its review"));
    s.sprints = vec![sprint(Phase::Review)];
    assert!(plan(&with, &PlanCtx { reviewing: Some(2), ..ctx(&s, &t, &p) }).is_ok());
}

#[test]
fn go_to_market_items_are_refused_for_a_product_that_isnt_sold() {
    let mut s = store();
    s.north_star = Some(NorthStar { name: "x".into(), metric: None, better: Better::Up, value: None, previous: None });
    s.market = Some(Market::Internal);
    let mut it = item_json("Launch on a community forum", "high", "M", "low");
    it["kind"] = json!("marketing");
    let (t, p) = (team(), vec![]);
    let why = plan(&json!({ "goal": "g", "done_when": "d", "items": [it.clone()] }), &ctx(&s, &t, &p)).unwrap_err();
    assert!(why.contains("isn't sold"), "{why}");
    s.market = Some(Market::Consumers);
    assert!(plan(&json!({ "goal": "g", "done_when": "d", "items": [it] }), &ctx(&s, &t, &p)).is_ok());
}

fn items(specs: &[(&str, Impact, Size, Risk, ItemKind)]) -> Vec<Item> {
    specs
        .iter()
        .enumerate()
        .map(|(n, (title, impact, size, risk, kind))| Item {
            id: n as u32 + 1,
            sprint: 1,
            kind: *kind,
            title: (*title).into(),
            change: String::new(),
            why: "w".into(),
            evidence: vec![],
            expected: "e".into(),
            impact: *impact,
            size: *size,
            risk: *risk,
            cost_tokens: cost(*size),
            owner: None,
            project: None,
            hypothesis: None,
            outside: false,
            rank: 0,
            recommended: false,
            may_auto_start: false,
            status: ItemStatus::Proposed,
            auto_started: false,
            ticket: None,
            experiment: None,
        })
        .collect()
}

#[test]
fn items_are_ranked_by_impact_against_cost_and_the_strongest_within_budget_recommended() {
    let mut all = items(&[
        ("Big rewrite", Impact::High, Size::L, Risk::High, ItemKind::New),
        ("Quick copy fix", Impact::Medium, Size::S, Risk::Low, ItemKind::Fix),
        ("Pricing page", Impact::High, Size::M, Risk::Low, ItemKind::New),
        ("Launch post", Impact::Medium, Size::S, Risk::Low, ItemKind::Marketing),
        ("Tidy logs", Impact::Low, Size::S, Risk::Low, ItemKind::Fix),
    ]);
    rank(&mut all, 10_000_000);
    let order: Vec<(&str, u32, bool)> = all.iter().map(|i| (i.title.as_str(), i.rank, i.recommended)).collect();
    assert_eq!(order, [("Quick copy fix", 1, true), ("Launch post", 2, true), ("Pricing page", 3, true), ("Tidy logs", 4, false), ("Big rewrite", 5, false)], "S+S+M = 10M fills the budget");
    let auto: Vec<&str> = all.iter().filter(|i| i.may_auto_start).map(|i| i.title.as_str()).collect();
    assert_eq!(auto, ["Quick copy fix", "Tidy logs"], "small, low-risk, not go-to-market");
    // Over budget, the first is still recommended; nothing more.
    let mut big = items(&[("Big", Impact::High, Size::L, Risk::Low, ItemKind::New), ("Other", Impact::Low, Size::L, Risk::Low, ItemKind::New)]);
    rank(&mut big, 1_000_000);
    assert_eq!(big.iter().map(|i| i.recommended).collect::<Vec<_>>(), [true, false]);
    // Auto-starting: recommended and small, only while there is budget.
    assert_eq!(auto_starting(&all, 0, 10_000_000).iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Quick copy fix"]);
    assert!(auto_starting(&all, 10_000_000, 10_000_000).is_empty(), "over budget: nothing on its own");
}

fn sprint(id: u32) -> Sprint {
    Sprint { id, goal: "More sign-ups".into(), done_when: "Short form live".into(), phase: Phase::Proposed, proposed_at: FRI_5, started_at: None, ends_at: None, closed_at: None, items: vec![1, 2], tickets: vec![], experiments: vec![], budget_tokens: 12_000_000, spent_tokens: 0, set: 0, retro: None, facts: None }
}

#[test]
fn the_person_starts_the_recommended_with_one_tap_or_ticks_their_own() {
    let mut all = items(&[("Quick copy fix", Impact::Medium, Size::S, Risk::Low, ItemKind::Fix), ("Pricing page", Impact::High, Size::L, Risk::Low, ItemKind::New)]);
    rank(&mut all, 3_000_000);
    all[1].outside = true;
    let refs: Vec<&Item> = all.iter().collect();
    let q = questions(&sprint(4), &refs, true, Some(Market::Businesses), Lang::En);
    assert_eq!(q.len(), 3);
    assert!(q[0].question.contains("Start sprint S-4?") && q[0].question.contains("Goal: More sign-ups") && q[0].question.contains("about 12M tokens") && q[0].question.contains("P-1 Quick copy fix"), "{}", q[0].question);
    assert!(q[0].options[0].recommended && q[0].options.len() == 3);
    assert!(q[1].multi_select && q[1].options[0].recommended && !q[1].options[1].recommended);
    assert!(q[1].options[1].description.contains("waits for your yes"), "{}", q[1].options[1].description);
    assert!(q[2].options[1].recommended && !q[2].options[0].recommended, "the PM's guess marked");
    // Korean words, not English, on a Korean card.
    let ko = questions(&sprint(4), &refs, false, None, Lang::Ko);
    assert_eq!(ko.len(), 2);
    assert!(ko[0].question.contains("목표") && ko[0].header == "스프린트 S-4", "{}", ko[0].question);

    let one_tap = picked(Some(&[Answer::pick(0)]), &refs, true);
    assert_eq!(one_tap, Picked { items: vec![1], market: None, not_now: false });
    let ticked = picked(Some(&[Answer::pick(0), Answer { picked: vec![1], ..Answer::default() }, Answer::pick(1)]), &refs, true);
    assert_eq!(ticked, Picked { items: vec![2], market: Some(Market::Businesses), not_now: false }, "ticks win");
    assert!(picked(Some(&[Answer::pick(2), Answer { picked: vec![0, 1], ..Answer::default() }]), &refs, false).not_now, "not now wins");
    assert!(picked(Some(&[Answer::pick(1)]), &refs, false).not_now, "start the ticked, none ticked");
    assert!(picked(None, &refs, false).not_now, "closed");
}

fn rule(pct: f64, sample: u64) -> Rule {
    Rule { better: Better::Up, min_change_pct: pct, window_days: 7, min_sample: sample }
}

#[test]
fn a_hypothesis_is_judged_by_its_rule_worked_didnt_or_inconclusive() {
    let r = |value, sample| Reading { value, sample };
    // A share: 18% of 2000 → 22% of 2000 is +22%, beyond chance (z ≈ 3.2).
    let (v, why, worse, pct) = judge(&rule(10.0, 200), true, r(18.0, 2000.0), r(22.0, 2000.0));
    assert_eq!((v, worse), (Verdict::Worked, false), "{why}");
    assert!((pct.unwrap() - 22.22).abs() < 0.01);
    // The same change on 60 people: too few.
    let (v, why, ..) = judge(&rule(10.0, 200), true, r(18.0, 60.0), r(21.0, 60.0));
    assert_eq!(v, Verdict::Inconclusive);
    assert!(why.contains("too few: 60 counted, the rule needs 200"), "{why}");
    // Enough people, but +17% on 250 each could be chance.
    let (v, why, ..) = judge(&rule(10.0, 200), true, r(18.0, 250.0), r(21.0, 250.0));
    assert_eq!(v, Verdict::Inconclusive, "{why}");
    assert!(why.contains("could be chance"));
    // Short of the rule: didn't.
    let (v, why, worse, _) = judge(&rule(10.0, 200), true, r(18.0, 5000.0), r(19.0, 5000.0));
    assert_eq!((v, worse), (Verdict::DidNot, false));
    assert!(why.contains("short of the +10%"), "{why}");
    // The wrong way, beyond chance: didn't, worse.
    let (v, _, worse, _) = judge(&rule(10.0, 200), true, r(20.0, 5000.0), r(15.0, 5000.0));
    assert_eq!((v, worse), (Verdict::DidNot, true));
    // A count: 400 → 480 events is +20%, z ≈ 2.7.
    assert_eq!(judge(&rule(10.0, 100), false, r(400.0, 400.0), r(480.0, 480.0)).0, Verdict::Worked);
    // Less is better: errors 300 → 200.
    let down = Rule { better: Better::Down, ..rule(20.0, 50) };
    assert_eq!(judge(&down, false, r(300.0, 300.0), r(200.0, 200.0)).0, Verdict::Worked);
    assert!(judge(&down, false, r(200.0, 200.0), r(300.0, 300.0)).2, "more errors: worse");
    // From nothing to something.
    assert_eq!(judge(&rule(10.0, 0), false, r(0.0, 0.0), r(50.0, 50.0)).0, Verdict::Worked);
}

#[test]
fn a_split_is_judged_by_its_best_variant_beside_control() {
    let arm = |v: &str, value, sample| Arm { variant: v.into(), value, sample };
    let arms = vec![arm("control", 18.0, 2000.0), arm("short", 22.0, 2000.0), arm("long", 17.0, 2000.0)];
    let (v, why, ..) = judge_split(&rule(10.0, 500), &arms, "control");
    assert_eq!(v, Verdict::Worked);
    assert!(why.starts_with("short beside control:"), "{why}");
    assert_eq!(judge_split(&rule(10.0, 500), &arms[1..], "control").0, Verdict::Inconclusive, "no control");
    assert_eq!(judge_split(&rule(10.0, 500), &arms[..1], "control").0, Verdict::Inconclusive, "no variant");
    assert_eq!(judge_split(&rule(10.0, 5000), &arms, "control").0, Verdict::Inconclusive, "each arm needs its sample");
}

fn hypothesis(metric: Metric, split: Option<Split>) -> Hypothesis {
    Hypothesis { belief: "A shorter form gets more sign-ups".into(), metric, rule: rule(10.0, 200), split, events: vec!["signup_started".into()] }
}

fn conversion() -> Metric {
    Metric::Conversion { from: "visit".into(), to: "signup".into(), days: 1, filter: String::new() }
}

#[tokio::test]
async fn a_hypothesis_is_read_through_any_product_data_and_judged_by_its_rule() {
    let live = FRI_5;
    let first = (live / DAY) as i64;
    // Before going live 18% of 2000; after, 22% of 2200.
    let fake = data::Fake::new(move |_, from, _| Ok(if from < first { Reading { value: 18.0, sample: 2000.0 } } else { Reading { value: 22.0, sample: 2200.0 } }));
    let mut x = experiment(1, 1, &hypothesis(conversion(), None), Some(7), Some(1), "pm", live - DAY);
    went_live(&mut x, live);
    assert!(!judge_due(&x, live + 6 * DAY) && judge_due(&x, live + 7 * DAY));
    assert!(check_due(&x, live + DAY) && !check_due(&x, live + 3600));
    let j = data::judge(&fake, &x, live + 7 * DAY).await;
    assert_eq!(j.verdict, Verdict::Worked, "{}", j.why);
    assert_eq!((j.before.unwrap().value, j.after.unwrap().sample), (18.0, 2200.0));
    assert_eq!((j.before_from.as_str(), j.to.as_str()), ("2026-10-30", "2026-11-12"));
    assert_eq!(j.source, "fake");
    x.result = Some(j);
    x.status = ExperimentStatus::Judged;
    let line = experiment_line(&x);
    assert!(line.starts_with("H-1 \"A shorter form gets more sign-ups\" · visit → signup within 1 day · rule: +10% in 7 days, at least 200: worked"), "{line}");
    assert!(line.contains("(18 → 22, 2000 counted)"), "{line}");

    // A split, where the source has flags.
    let split = Split { flag: "short-form".into(), control: "control".into() };
    let flagged = data::Fake::new(|_, _, _| Err("not this way".into())).with_split(|s, _, _, _| {
        assert_eq!(s.flag, "short-form");
        Ok(vec![Arm { variant: "control".into(), value: 18.0, sample: 3000.0 }, Arm { variant: "test".into(), value: 21.0, sample: 3000.0 }])
    });
    let mut ab = experiment(2, 1, &hypothesis(conversion(), Some(split)), Some(8), None, "data", live);
    went_live(&mut ab, live);
    let j = data::judge(&flagged, &ab, live + 7 * DAY).await;
    assert_eq!((j.verdict, j.arms.len()), (Verdict::Worked, 2), "{}", j.why);
    // …and where it has none: before beside after, said so.
    let j = data::judge(&fake, &ab, live + 7 * DAY).await;
    assert!(j.why.contains("not A/B"), "{}", j.why);
    // Numbers that can't be read: inconclusive, saying why.
    let broken = data::Fake::new(|_, _, _| Err("PostHog said 401".into()));
    let j = data::judge(&broken, &x, live + 7 * DAY).await;
    assert_eq!(j.verdict, Verdict::Inconclusive);
    assert!(j.why.contains("PostHog said 401"));
    // The day after it went live: are its events arriving?
    assert!(data::arriving(&fake, &x).await.unwrap());
    let silent = data::Fake::new(|_, _, _| Ok(Reading { value: 0.0, sample: 0.0 }));
    assert!(!data::arriving(&silent, &x).await.unwrap());
}

#[test]
fn posthog_reads_a_metric_with_its_filter_and_splits_by_the_flags_variants() {
    let q = data::metric_query(&Metric::Event { event: "signup".into(), filter: "utm_source=news".into() }, 20_400, 20_406, " AND NOT (x)");
    assert!(q.contains("event = 'signup'") && q.contains(" AND NOT (x) AND coalesce(toString(properties.utm_source), '') = 'news'"), "{q}");
    let q = data::metric_query(&conversion(), 20_400, 20_406, "");
    assert!(q.contains("countIf(arrayExists"), "{q}");
    let s = data::split_query("short-form", &conversion(), 20_400, 20_406, "").unwrap();
    assert!(s.contains("properties['$feature/short-form']") && s.contains("GROUP BY v"), "{s}");
    assert!(crate::metrics::hogql::select_only(&s).is_ok(), "one read-only SELECT: {s}");
    assert!(data::split_query("x", &Metric::Event { event: "a".into(), filter: String::new() }, 0, 1, "").is_none());
    let arms = data::arms(&[vec![json!("control"), json!(1000), json!(180)], vec![json!("test"), json!("1000"), json!(210)]]);
    assert_eq!(arms, [Arm { variant: "control".into(), value: 18.0, sample: 1000.0 }, Arm { variant: "test".into(), value: 21.0, sample: 1000.0 }]);
}

#[test]
fn a_started_items_ticket_carries_its_hypothesis_events_and_rule() {
    let mut it = items(&[("Shorter signup form", Impact::High, Size::S, Risk::Low, ItemKind::Iterate)]).remove(0);
    it.evidence = vec!["E1 · signups: 18%".into()];
    it.outside = true;
    let x = experiment(5, 3, &hypothesis(conversion(), Some(Split { flag: "short-form".into(), control: "control".into() })), None, Some(1), "pm", FRI_5);
    let (title, body) = ticket_draft(&Item { sprint: 3, ..it.clone() }, Some(&x));
    assert_eq!(title, "S-3 P-1: Shorter signup form");
    for must in ["Why: w", "It stands on:\n- E1 · signups: 18%", "Hypothesis H-5: A shorter form gets more sign-ups", "judged 7 days after it is live", "moves +10% or more", "at least 200 counted", "`$feature/short-form`", "`signup_started`, `visit`, `signup`", DEFINE, "waits for the person's yes"] {
        assert!(body.contains(must), "{must}\n{body}");
    }
    let (_, plain) = ticket_draft(&items(&[("x", Impact::Low, Size::S, Risk::Low, ItemKind::Fix)])[0], None);
    assert!(!plain.contains("Hypothesis"));
}

#[test]
fn the_review_hands_the_pm_its_facts_results_and_numbers_in_one_compact_message() {
    let mut s = sprint(3);
    s.phase = Phase::Active;
    let mut x = experiment(2, 3, &hypothesis(conversion(), None), Some(12), Some(1), "pm", FRI_5);
    went_live(&mut x, FRI_5 - 8 * DAY);
    x.status = ExperimentStatus::Judged;
    x.result = Some(Judged { verdict: Verdict::DidNot, why: "moved +2%, short of the +10% the rule asked".into(), worse: false, before: Some(Reading { value: 18.0, sample: 900.0 }), after: Some(Reading { value: 18.4, sample: 950.0 }), arms: vec![], change_pct: Some(2.2), from: "a".into(), to: "b".into(), before_from: String::new(), before_to: String::new(), query: String::new(), source: "fake".into(), at: FRI_5 });
    let facts = Facts { tickets: 4, done: 3, shipped: 3, troubles: 2, waits: 1, waited_secs: 3600, worked: 0, did_not: 1, inconclusive: 0, tokens: 9_000_000 };
    let st = store();
    let north = NorthStar { name: "weekly sign-ups".into(), metric: Some(Metric::Event { event: "signup".into(), filter: String::new() }), better: Better::Up, value: Some(130.0), previous: Some(120.0) };
    let review = Review {
        sprint: Some(&s),
        from: "2026-10-31".into(),
        to: "2026-11-06".into(),
        tickets: vec!["T-12 ✓ Short form".into()],
        experiments: vec![experiment_line(&x)],
        tried: tried(std::slice::from_ref(&x)),
        north_star: Some(&north),
        facts: Some(&facts),
        budget: 12_000_000,
        troubles: vec!["sent back · Short form: sent back by you".into()],
        friction: vec!["frontend (frontend): 1 snag(s), waited on the person 1×".into()],
        evidence: &st.evidence,
        gaps: vec![],
        market: Some(Market::Consumers),
        marketer: true,
        measurable: true,
        later: vec!["P-9 Google sign-in — +5%".into()],
        platform: false,
        notes: vec![],
    };
    let text = review_message(&review);
    for must in [
        "[Sprint review · S-3 · 2026-10-31 – 2026-11-06]",
        "`retro`: shipped, learned, wrong, next",
        "Goal: More sign-ups",
        "Tickets: 3 of 4 done · shipped 3 · rubbed 2 · waited on the person 1× (1h 00m) · used 9M of 12M tokens",
        "- T-12 ✓ Short form",
        "H-2 \"A shorter form gets more sign-ups\"",
        "didn't — moved +2%",
        "What rubbed (equipe's own flow, the team's inputs):",
        "North star: weekly sign-ups · signup events: 130 (before: 120)",
        "- E1 · `funnel:visit>signup` ·",
        "Tried before",
        "Left for later by the person",
        "Go-to-market: sold to people (B2C). The marketing member's work belongs in the plan",
        "always wait for the person's card",
    ] {
        assert!(text.contains(must), "{must}\n---\n{text}");
    }
    assert!(!text.contains("query:") && !text.contains("SELECT"), "no HogQL in the PM's message: {text}");
    assert!(text.chars().count() < 4000, "compact: {} characters", text.chars().count());

    // No sprint before: a planning, asking the person whether it's sold, saying what isn't measured.
    let first = planning_message(&Review { to: "2026-11-06".into(), market: None, measurable: false, ..Review::default() });
    for must in ["[Sprint planning · 2026-11-06]", "Name the `north_star` once", "Go-to-market: not said yet. equipe asks the person", "No product analytics are connected", "kind `measure`"] {
        assert!(first.contains(must), "{must}\n---\n{first}");
    }
    let sold_alone = planning_message(&Review { market: Some(Market::Businesses), marketer: false, measurable: true, ..Review::default() });
    assert!(sold_alone.contains("propose adding the marketing role"), "{sold_alone}");
}

#[test]
fn the_pms_turns_carry_the_sprint_in_a_few_lines() {
    let mut st = Store::default();
    assert_eq!(context(&st, 0, FRI_5), None);
    let mut s = sprint(3);
    s.phase = Phase::Active;
    s.ends_at = Some(FRI_5 + 2 * DAY);
    s.tickets = vec![1, 2, 3];
    s.experiments = vec![1];
    s.spent_tokens = 4_000_000;
    st.sprints.push(s);
    st.experiments.push(experiment(1, 3, &hypothesis(conversion(), None), Some(1), None, "pm", FRI_5));
    let text = context(&st, 1, FRI_5).unwrap();
    assert_eq!(text, "The squad's sprint (docs: `plan_sprint`):\nS-3 (review in 2d 0h): More sign-ups\nDone when: Short form live · 1 of 3 tickets done · 4M of 12M tokens\nH-1 A shorter form gets more sign-ups: not live");
    assert!(text.lines().count() <= 5);
}

#[test]
fn budget_levels_are_told_once_each() {
    assert_eq!(budget_step(0, 7_000_000, 10_000_000), None);
    assert_eq!(budget_step(0, 8_000_000, 10_000_000), Some(80));
    assert_eq!(budget_step(80, 9_000_000, 10_000_000), None);
    assert_eq!(budget_step(80, 10_000_000, 10_000_000), Some(100));
    assert_eq!(budget_step(0, 12_000_000, 10_000_000), Some(100), "past both at once: the higher");
    assert_eq!(budget_step(100, 20_000_000, 10_000_000), None);
}

#[test]
fn a_sprints_facts_count_its_tickets_and_results() {
    let mut s = sprint(1);
    s.tickets = vec![1, 2];
    s.experiments = vec![1, 2];
    let board: Vec<Ticket> = [(1, TicketStatus::Done), (2, TicketStatus::InProgress), (3, TicketStatus::Done)].iter().map(|(id, status)| ticket(*id, *status)).collect();
    let judged = |id, verdict| {
        let mut x = experiment(id, 1, &hypothesis(conversion(), None), None, None, "pm", 0);
        x.result = Some(Judged { verdict, why: String::new(), worse: false, before: None, after: None, arms: vec![], change_pct: None, from: String::new(), to: String::new(), before_from: String::new(), before_to: String::new(), query: String::new(), source: String::new(), at: 0 });
        x
    };
    let f = facts(&s, &board, &crate::retro::Facts::default(), &[judged(1, Verdict::Worked), judged(2, Verdict::Inconclusive), judged(3, Verdict::DidNot)], 5);
    assert_eq!((f.tickets, f.done, f.worked, f.did_not, f.inconclusive, f.tokens), (2, 1, 1, 0, 1, 5));
}

#[test]
fn the_loop_is_kept_and_read_back() {
    let dir = std::env::temp_dir().join(format!("equipe-sprint-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(load(&dir), Store::default());
    let mut st = store();
    st.sprints.push(sprint(1));
    st.items = items(&[("a", Impact::Low, Size::S, Risk::Low, ItemKind::Fix)]);
    st.count_turn(FRI_5, 30_000);
    save(&dir, &st).unwrap();
    assert_eq!(load(&dir), st);
    assert_eq!(st.loop_spent(FRI_5 + DAY), 30_000);
    assert_eq!(st.loop_spent(FRI_5 + 8 * DAY), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
    Ticket { summary: None,
        id,
        title: format!("t{id}"),
        body: String::new(),
        status,
        kind: equipe_protocol::TicketKind::Code,
        owner: Some("frontend".into()),
        assignee: None,
        project: None,
        depends_on: vec![],
        branch: None,
        worktree: None,
        execution: None,
        merged: None,
        off_main: None,
        design_qa: None,
        landing: false,
        created_by: "pm".into(),
        note: None,
        plan: None,
        model: None,
        flow: None,
        asks: vec![],
        label: None,
        remaining: vec![],
        images: Vec::new(),
        updated_at: None,
        started_at: None,
        app: None,
        priority: Default::default(),
        rank: 0,
    }
}

#[test]
fn a_platform_team_is_told_how_to_measure_with_its_experiments() {
    let mut r = Review { to: "2026-11-06".into(), measurable: true, notes: vec!["Top sources (visitors): news.example (40).".into()], ..Review::default() };
    let plain = planning_message(&r);
    assert!(!plain.contains("equipe platform") && !plain.contains("create_experiment"));
    r.platform = true;
    let text = planning_message(&r);
    assert!(text.contains("Top sources (visitors): news.example (40)."));
    assert!(text.contains("`split.flag`") && text.contains("`create_experiment`") && text.contains("`equipe.variant(key)`"));
}
