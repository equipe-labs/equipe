use super::*;

const NOW: i64 = 1_000_000;
const H: i64 = 3_600;
const D: i64 = 24 * H;
/// Points a second for points an hour.
fn per_hour(p: f64) -> f64 {
    p / H as f64
}

fn budget(used: f64, left: i64, burn_per_hour: f64, runs: usize) -> Budget {
    Budget { used, resets_at: NOW + left, burn: Some(per_hour(burn_per_hour)), runs, other_back: None, age: 60 }
}

fn auto() -> Parallel {
    Parallel::default()
}

#[test]
fn burn_above_what_the_week_can_sustain_lowers_the_runs_in_proportion() {
    // 40% used, three days left: 50 points (10 kept back) over 72 hours is 0.69 points an hour.
    let b = budget(40., 3 * D, 1.5, 8);
    let (rate, until) = sustainable(&b, NOW, None).unwrap();
    assert!((rate - per_hour(50. / 72.)).abs() < 1e-9, "{rate}");
    assert_eq!(until, Until::Reset(NOW + 3 * D));
    // 1.5 an hour on 8 runs: 8 × 0.69 / 1.5 = 3.7 → 3 runs at once, of the 10 the lane policy allows.
    let (state, change) = step(State::default(), 10, &b, &auto(), NOW);
    assert_eq!(state, State { cap: Some(3), since: NOW });
    assert_eq!(change, Some(Change::Lowered { to: 3, of: 10, until: Until::Reset(NOW + 3 * D) }));
    assert_eq!(capped(10, state), 3);
    // A burn the week sustains changes nothing.
    assert_eq!(step(State::default(), 10, &budget(40., 3 * D, 0.6, 8), &auto(), NOW), (State::default(), None));
}

#[test]
fn a_fresh_week_burned_at_ten_percent_in_ninety_minutes_is_paced_to_the_least() {
    // The squad that went 0 → 10% in 1.5 hours with a week to go: its burn is 14 times what lasts.
    let b = budget(10., 166 * H + 30 * 60, 10. / 1.5, 10);
    let (state, change) = step(State::default(), 10, &b, &auto(), NOW);
    assert_eq!(state.cap, Some(LEAST), "never under one run — the PM's and the most urgent ticket's");
    assert!(matches!(change, Some(Change::Lowered { to: 1, of: 10, .. })));
}

#[test]
fn it_never_goes_below_one_run_and_never_above_what_the_lane_policy_allows() {
    let b = budget(95., 6 * D, 5., 4);
    assert_eq!(step(State::default(), 10, &b, &auto(), NOW).0.cap, Some(1), "nothing to spare: one run");
    assert_eq!(capped(2, State { cap: Some(9), since: 0 }), 2, "a cap above the policy is the policy");
    assert_eq!(capped(0, State { cap: Some(1), since: 0 }), 0);
    assert_eq!(capped(7, State::default()), 7);
}

#[test]
fn the_cap_rises_only_after_the_dwell_and_only_when_the_burn_is_well_under_the_rate() {
    let paced = |ago: i64| State { cap: Some(3), since: NOW - ago };
    // Rate: 0.69 points an hour. Burn 0.3: well under it.
    let low = budget(40., 3 * D, 0.3, 3);
    assert_eq!(step(paced(10 * 60), 10, &low, &auto(), NOW), (paced(10 * 60), None), "ten minutes in: the dwell holds it");
    let (state, change) = step(paced(MIN_DWELL), 10, &low, &auto(), NOW);
    assert_eq!(state, State { cap: Some(5), since: NOW }, "half again at most per step: 3 → 5");
    assert!(matches!(change, Some(Change::Raised { to: 5, of: 10, .. })));
    // Burn 0.6 is under the rate but not 20% under: held, not flapped.
    let near = budget(40., 3 * D, 0.6, 3);
    assert_eq!(step(paced(H), 10, &near, &auto(), NOW), (paced(H), None));
    // The cap isn't binding (fewer runs at work than it allows): nothing to give back.
    let idle = budget(40., 3 * D, 0.3, 1);
    assert_eq!(step(paced(H), 10, &idle, &auto(), NOW), (paced(H), None));
}

#[test]
fn the_cap_lowers_again_only_after_the_dwell() {
    let paced = |ago: i64| State { cap: Some(6), since: NOW - ago };
    let hot = budget(40., 3 * D, 3., 6);
    assert_eq!(step(paced(5 * 60), 10, &hot, &auto(), NOW), (paced(5 * 60), None), "just changed: the burn still shows the old runs");
    let (state, change) = step(paced(MIN_DWELL), 10, &hot, &auto(), NOW);
    assert_eq!(state.cap, Some(1), "6 × 0.69 / 3 = 1.4");
    assert!(matches!(change, Some(Change::Lowered { to: 1, .. })));
}

#[test]
fn it_lifts_when_the_burn_falls_far_enough_that_the_policy_cap_is_sustainable() {
    let paced = State { cap: Some(8), since: NOW - H };
    let (state, change) = step(paced, 10, &budget(40., 3 * D, 0.2, 8), &auto(), NOW);
    assert_eq!(state, State { cap: None, since: NOW });
    assert!(matches!(change, Some(Change::Lifted { .. })));
}

#[test]
fn the_other_engine_out_until_t_makes_the_plan_carry_the_squad_to_t() {
    let reset = NOW + 5 * D;
    let mut b = Budget { used: 85., resets_at: reset, burn: Some(per_hour(0.6)), runs: 2, other_back: Some(NOW + D), age: 60 };
    let per_run = per_hour(0.3);
    // Lasting until the reset would leave 5 points over 5 days: 0.04 an hour, under what one run burns (0.3).
    let own = per_hour(5. / 120.);
    let (rate, until) = sustainable(&b, NOW, Some(per_run)).unwrap();
    assert_eq!(until, Until::OtherBack(NOW + D), "one run can't last to the reset: to the other's return");
    assert!((rate - per_hour(13. / 24.)).abs() < 1e-9 && rate > own, "13 points over the day: {rate}");
    // At that rate two runs at 0.5 an hour are fine: no pacing.
    let calm = Budget { burn: Some(per_hour(0.5)), ..b };
    assert_eq!(step(State::default(), 4, &calm, &auto(), NOW).1, None);
    // Four runs at 2 an hour are not: 4 × 0.54 / 2 = 1.08 → one run.
    let hot = Budget { burn: Some(per_hour(2.0)), runs: 4, ..b };
    let (state, change) = step(State::default(), 4, &hot, &auto(), NOW);
    assert_eq!(state.cap, Some(1));
    assert!(matches!(change, Some(Change::Lowered { until: Until::OtherBack(_), .. })), "{change:?}");
    // When one run can last to the reset, that stays the aim.
    b.used = 40.;
    assert_eq!(sustainable(&b, NOW, Some(per_hour(0.01))).unwrap().1, Until::Reset(reset));
    // The other engine back after this week starts over doesn't matter.
    b.other_back = Some(reset + D);
    assert_eq!(sustainable(&b, NOW, Some(per_run)).unwrap().1, Until::Reset(reset));
}

#[test]
fn the_person_choosing_max_ends_pacing_and_it_stays_off() {
    let paced = State { cap: Some(2), since: NOW - H };
    let hot = budget(40., 3 * D, 3., 8);
    assert_eq!(step(paced, 10, &hot, &Parallel::max(6, 16), NOW), (State::default(), None));
    assert_eq!(step(State::default(), 10, &hot, &Parallel::max(6, 16), NOW), (State::default(), None), "never starts on max");
    assert_ne!(step(State::default(), 10, &hot, &auto(), NOW).0.cap, None, "auto again: paced again");
}

#[test]
fn a_stale_reading_an_unknown_burn_or_a_week_nearly_over_changes_nothing_new() {
    let paced = State { cap: Some(3), since: NOW - H };
    let mut b = budget(40., 3 * D, 5., 8);
    b.age = STALE_AFTER + 1;
    assert_eq!(step(State::default(), 10, &b, &auto(), NOW), (State::default(), None));
    assert_eq!(step(paced, 10, &b, &auto(), NOW), (paced, None), "kept as it is");
    b.age = 0;
    b.burn = None;
    assert_eq!(step(paced, 10, &b, &auto(), NOW), (paced, None));
    // The week started over: pacing ends.
    let over = Budget { resets_at: NOW - 1, ..budget(5., 0, 5., 8) };
    let (state, change) = step(paced, 10, &over, &auto(), NOW);
    assert_eq!(state.cap, None);
    assert!(matches!(change, Some(Change::Lifted { .. })));
    // Under an hour left: not worth holding anything back.
    let (state, change) = step(paced, 10, &budget(98., 30 * 60, 5., 8), &auto(), NOW);
    assert_eq!((state.cap, change.map(|c| c.kind())), (None, Some(kind::OFF)));
    assert_eq!(step(State::default(), 10, &budget(98., 30 * 60, 5., 8), &auto(), NOW), (State::default(), None));
}

#[test]
fn a_run_paused_for_a_limit_holds_no_slot_while_paced() {
    assert!(holds_slot(false, true) && holds_slot(false, false));
    assert!(!holds_slot(true, true), "paced: a paused run burns nothing");
    assert!(holds_slot(true, false), "not paced: as before");
}

#[test]
fn every_change_has_its_journal_kind() {
    let until = Until::Reset(1);
    assert_eq!(Change::Lowered { to: 1, of: 2, until }.kind(), "paced_down");
    assert_eq!(Change::Raised { to: 2, of: 3, until }.kind(), "paced_up");
    assert_eq!(Change::Lifted { until }.kind(), "paced_off");
}
