use super::*;

const NOW: i64 = 1_000_000;
const H: i64 = 3_600;
const D: i64 = 24 * H;

fn budget(used: f64, left: i64) -> Budget {
    Budget { used, resets_at: NOW + left, other_back: None, age: 60 }
}

fn auto() -> Parallel {
    Parallel::default()
}

fn held() -> State {
    State { held: true, since: NOW - H }
}

#[test]
fn nothing_is_held_back_while_more_than_the_reserve_is_left() {
    // The real squad's case: 0 to 10% in ninety minutes, a week to go. Full speed.
    let fresh = budget(10., 166 * H);
    assert_eq!(step(State::default(), &fresh, &auto(), NOW), (State::default(), None));
    // Even at 80% used (20 left) nothing changes, however fast it got there.
    assert_eq!(step(State::default(), &budget(80., 3 * D), &auto(), NOW), (State::default(), None));
    // Exactly the reserve is still enough room.
    assert_eq!(step(State::default(), &budget(100. - RESERVE, 3 * D), &auto(), NOW).1, None);
}

#[test]
fn under_the_reserve_before_the_reset_only_the_pm_and_urgent_work_start() {
    let b = budget(86., 3 * D);
    let (state, change) = step(State::default(), &b, &auto(), NOW);
    assert_eq!(state, State { held: true, since: NOW });
    assert_eq!(change, Some(Change::Held { until: Until::Reset(NOW + 3 * D) }));
    assert_eq!(change.unwrap().kind(), "reserve_on");
    // It stays held on the next reading, and tells nothing again.
    assert_eq!(step(state, &b, &auto(), NOW + 60), (state, None));
}

#[test]
fn the_other_engine_out_until_it_is_back_before_the_reset_sets_the_horizon() {
    let b = Budget { other_back: Some(NOW + D), ..budget(90., 5 * D) };
    assert_eq!(horizon(&b, NOW), Some(Until::OtherBack(NOW + D)));
    let (_, change) = step(State::default(), &b, &auto(), NOW);
    assert_eq!(change, Some(Change::Held { until: Until::OtherBack(NOW + D) }));
    // Back only after this week starts over: the reset is the horizon.
    let late = Budget { other_back: Some(NOW + 6 * D), ..b };
    assert_eq!(horizon(&late, NOW), Some(Until::Reset(NOW + 5 * D)));
    // Already back: ignored.
    let back = Budget { other_back: Some(NOW - 1), ..b };
    assert_eq!(horizon(&back, NOW), Some(Until::Reset(NOW + 5 * D)));
}

#[test]
fn the_hold_lifts_when_room_is_back_but_not_on_a_wobble_around_the_line() {
    // 14.5 left: still held. 15.5: inside the margin, still held. 16: lifted.
    assert_eq!(step(held(), &budget(85.5, 3 * D), &auto(), NOW), (held(), None));
    assert_eq!(step(held(), &budget(84.5, 3 * D), &auto(), NOW), (held(), None), "not flapping at the line");
    let (state, change) = step(held(), &budget(84., 3 * D), &auto(), NOW);
    assert_eq!(state, State { held: false, since: NOW });
    assert_eq!(change.map(|c| c.kind()), Some("reserve_off"));
}

#[test]
fn the_hold_ends_when_the_window_starts_over_or_little_time_is_left() {
    let (state, change) = step(held(), &Budget { resets_at: NOW - 1, ..budget(5., 0) }, &auto(), NOW);
    assert_eq!((state.held, change.map(|c| c.kind())), (false, Some("reserve_off")), "the week started over");
    // Under an hour to the reset: the little left is for using.
    let (state, change) = step(held(), &budget(98., 30 * 60), &auto(), NOW);
    assert_eq!((state.held, change.map(|c| c.kind())), (false, Some("reserve_off")));
    assert_eq!(step(State::default(), &budget(98., 30 * 60), &auto(), NOW), (State { held: false, since: NOW }, None), "and it doesn't start then");
    // The other engine back within the hour ends it too.
    let soon = Budget { other_back: Some(NOW + 20 * 60), ..budget(98., 3 * D) };
    assert!(!step(held(), &soon, &auto(), NOW).0.held);
}

#[test]
fn a_stale_reading_changes_nothing() {
    let stale = Budget { age: STALE_AFTER + 1, ..budget(97., 3 * D) };
    assert_eq!(step(State::default(), &stale, &auto(), NOW), (State::default(), None));
    assert_eq!(step(held(), &Budget { age: STALE_AFTER + 1, ..budget(10., 3 * D) }, &auto(), NOW), (held(), None), "kept as it is");
}

#[test]
fn the_person_choosing_max_ends_it_and_it_stays_off() {
    let low = budget(97., 3 * D);
    assert_eq!(step(held(), &low, &Parallel::max(6, 16), NOW), (State::default(), None));
    assert_eq!(step(State::default(), &low, &Parallel::max(6, 16), NOW), (State::default(), None), "never starts on max");
    assert!(step(State::default(), &low, &auto(), NOW).0.held, "auto again: held again");
}

#[test]
fn only_urgent_and_high_tickets_are_admitted_under_the_reserve() {
    assert!(admitted(Priority::Urgent) && admitted(Priority::High));
    assert!(!admitted(Priority::Normal) && !admitted(Priority::Low));
}

#[test]
fn a_run_paused_for_a_limit_holds_no_slot_while_the_reserve_is_held() {
    assert!(holds_slot(false, true) && holds_slot(false, false));
    assert!(!holds_slot(true, true), "held: a paused run burns nothing");
    assert!(holds_slot(true, false), "not held: as before");
}

#[test]
fn every_change_has_its_journal_kind() {
    let until = Until::Reset(1);
    assert_eq!(Change::Held { until }.kind(), "reserve_on");
    assert_eq!(Change::Lifted { until }.kind(), "reserve_off");
}
