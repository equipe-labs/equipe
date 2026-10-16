//! The switching rules, edge by edge: every threshold, the five-hour
//! exception, both engines low, stale readings, coming back without flapping,
//! and the order roles move in.

use super::*;
use equipe_protocol::engines::{Switching, Why, Window};
use equipe_protocol::{Engine, PlanWindow};

const NOW: i64 = 1_760_000_000;
const M: i64 = 60;
const H: i64 = 3600;

fn win(used: u8, left: i64) -> Option<PlanWindow> {
    Some(PlanWindow { used_percent: used, resets_at: NOW + left })
}

/// A reading of `engine` taken at `at`: five hours used and left, a week used and left.
fn read_at(engine: Engine, at: i64, five: Option<(u8, i64)>, week: Option<(u8, i64)>) -> Reading {
    Reading { engine, five_hour: five.and_then(|(u, l)| win(u, l)), weekly: week.and_then(|(u, l)| win(u, l)), at, warning: false, rejected: false }
}

fn claude(five: u8, week: u8) -> Reading {
    read_at(Engine::Claude, NOW, Some((five, 2 * H)), Some((week, 3 * 24 * H)))
}

fn codex(five: u8, week: u8) -> Reading {
    read_at(Engine::Codex, NOW, Some((five, 3 * H)), Some((week, 4 * 24 * H)))
}

fn mate(name: &str, engine: Engine) -> Mate {
    let role = name.split('-').next().unwrap_or(name);
    Mate {
        name: name.into(),
        role: role.into(),
        engine,
        home: engine,
        home_model: None,
        model: None,
        effort: None,
        edits: crate::roles::role_or_custom(role).edits,
        pinned: false,
        switched: Vec::new(),
        waiting: false,
    }
}

fn team(engine: Engine) -> Vec<Mate> {
    ["pm", "designer", "frontend", "backend", "mobile", "qa", "data"].iter().map(|n| mate(n, engine)).collect()
}

/// Both engines ready, and Codex able to do everything Claude does.
fn reach() -> Reach {
    Reach { ready: vec![Engine::Claude, Engine::Codex], codex_edits: true, codex_board: true }
}

fn decide_at(readings: &[Reading], mates: &[Mate], reach: &Reach, mode: Switching, now: i64, memory: &mut Memory) -> Decision {
    for r in readings {
        memory.observe(r);
    }
    decide(&Squad { readings, mates, reach, mode, now }, memory)
}

fn run(readings: &[Reading], mates: &[Mate]) -> Decision {
    decide_at(readings, mates, &reach(), Switching::Auto, NOW, &mut Memory::default())
}

fn moved(d: &Decision) -> Vec<&str> {
    d.batch.as_ref().map(|b| b.moves.iter().map(|m| m.member.as_str()).collect()).unwrap_or_default()
}

fn level(five: Option<(u8, i64)>, week: Option<(u8, i64)>) -> (Level, bool) {
    let a = assess(&read_at(Engine::Claude, NOW, five, week), &Memory::default(), NOW);
    (a.level, a.wait)
}

#[test]
fn every_threshold_on_its_edge() {
    use Level::*;
    let cases: &[(&str, Option<(u8, i64)>, Option<(u8, i64)>, Level, bool)] = &[
        ("nothing read", None, None, Ok, false),
        ("five hours 69", Some((69, 3 * H)), None, Ok, false),
        ("five hours 70", Some((70, 3 * H)), None, Watch, false),
        ("five hours 84", Some((84, 3 * H)), None, Watch, false),
        ("five hours 85, 31 minutes left", Some((85, 31 * M)), None, Shift, false),
        ("five hours 85, 30 minutes left: watched, not shifted", Some((85, 30 * M)), None, Watch, false),
        ("five hours 94", Some((94, 3 * H)), None, Shift, false),
        ("five hours 95", Some((95, 3 * H)), None, Drain, false),
        ("five hours 99", Some((99, 3 * H)), None, Drain, false),
        ("five hours 100", Some((100, 3 * H)), None, Exhausted, false),
        ("five hours past their reset", Some((99, -M)), None, Ok, false),
        ("a week 69", None, Some((69, 3 * 24 * H)), Ok, false),
        ("a week 70", None, Some((70, 3 * 24 * H)), Watch, false),
        ("a week 84", None, Some((84, 3 * 24 * H)), Watch, false),
        ("a week 85", None, Some((85, 3 * 24 * H)), Shift, false),
        ("a week 85, an hour left: no exception for a week", None, Some((85, H)), Shift, false),
        ("a week 91", None, Some((91, 3 * 24 * H)), Shift, false),
        ("a week 92", None, Some((92, 3 * 24 * H)), Drain, false),
        ("a week 100", None, Some((100, 24 * H)), Exhausted, false),
        ("the worst window wins", Some((40, 3 * H)), Some((93, 24 * H)), Drain, false),
    ];
    for (what, five, week, want, wait) in cases {
        assert_eq!(level(*five, *week), (*want, *wait), "{what}");
    }
}

#[test]
fn five_hours_about_to_start_over_are_waited_for_while_the_week_has_room() {
    let cases: &[(&str, u8, i64, Option<u8>, bool)] = &[
        ("95, 30 minutes left, week 74: wait", 95, 30 * M, Some(74), true),
        ("95, 30 minutes left, week 75: move", 95, 30 * M, Some(75), false),
        ("95, 31 minutes left: move", 95, 31 * M, Some(10), false),
        ("100, 10 minutes left, no week: wait", 100, 10 * M, None, true),
        ("99, 5 minutes left, week 80: move", 99, 5 * M, Some(80), false),
    ];
    for (what, used, left, week, wait) in cases {
        let (_, waits) = level(Some((*used, *left)), week.map(|w| (w, 3 * 24 * H)));
        assert_eq!(waits, *wait, "{what}");
    }
    // Waited for: no new tickets until the reset, and nothing moves.
    let r = [read_at(Engine::Claude, NOW, Some((96, 20 * M)), Some((40, 3 * 24 * H))), codex(5, 5)];
    let d = run(&r, &team(Engine::Claude));
    assert!(d.batch.is_none(), "{d:?}");
    assert_eq!(d.hold, vec![Hold { engine: Engine::Claude, until: NOW + 20 * M }]);
    assert!(d.notices.iter().any(|n| n.engine == Engine::Claude && n.wait));
    // The week has no room to spare: they move.
    let r = [read_at(Engine::Claude, NOW, Some((96, 20 * M)), Some((80, 3 * 24 * H))), codex(5, 5)];
    assert_eq!(run(&r, &team(Engine::Claude)).batch.map(|b| b.why), Some(Why::Drain));
}

#[test]
fn the_engines_own_word_counts() {
    let mut r = claude(40, 20);
    r.warning = true;
    assert_eq!(assess(&r, &Memory::default(), NOW).level, Level::Shift, "allowed_warning shifts");
    r.rejected = true;
    let a = assess(&r, &Memory::default(), NOW);
    assert_eq!((a.level, a.window), (Level::Exhausted, Some(Window::FiveHour)));
}

#[test]
fn the_burn_so_far_says_when_a_window_runs_out() {
    let mut memory = Memory::default();
    // Ten points in ten minutes: a point a minute, 40 left with two hours to go.
    memory.observe(&read_at(Engine::Claude, NOW - 10 * M, Some((50, 2 * H)), None));
    memory.observe(&read_at(Engine::Claude, NOW, Some((60, 2 * H)), None));
    let a = assess(&read_at(Engine::Claude, NOW, Some((60, 2 * H)), None), &memory, NOW);
    assert_eq!(a.level, Level::Watch, "under 70, but out before the reset");
    let out = a.runs_out_at.expect("runs out");
    assert!((NOW + 39 * M..=NOW + 41 * M).contains(&out), "{}", out - NOW);
    // Slow enough to last: nothing to say.
    let mut slow = Memory::default();
    slow.observe(&read_at(Engine::Claude, NOW - 60 * M, Some((50, 2 * H)), None));
    slow.observe(&read_at(Engine::Claude, NOW, Some((51, 2 * H)), None));
    assert_eq!(assess(&read_at(Engine::Claude, NOW, Some((51, 2 * H)), None), &slow, NOW).level, Level::Ok);
    // A week at 76 with three days left, burning through it: shifts early.
    let mut week = Memory::default();
    week.observe(&read_at(Engine::Claude, NOW - H, None, Some((70, 3 * 24 * H))));
    week.observe(&read_at(Engine::Claude, NOW, None, Some((76, 3 * 24 * H))));
    assert_eq!(assess(&read_at(Engine::Claude, NOW, None, Some((76, 3 * 24 * H))), &week, NOW).level, Level::Shift);
    // A new window forgets the old one's burn.
    week.observe(&read_at(Engine::Claude, NOW + M, None, Some((1, 7 * 24 * H))));
    assert_eq!(assess(&read_at(Engine::Claude, NOW + M, None, Some((1, 7 * 24 * H))), &week, NOW + M).runs_out_at, None);
}

#[test]
fn a_shift_moves_the_implementation_roles_in_order_and_a_drain_everyone_the_pm_last() {
    let d = run(&[claude(86, 30), codex(10, 10)], &team(Engine::Claude));
    let batch = d.batch.clone().expect("a batch");
    assert_eq!((batch.why, batch.engine, batch.window, batch.used), (Why::Shift, Engine::Claude, Some(Window::FiveHour), 86));
    assert_eq!(moved(&d), ["backend", "mobile", "frontend"]);
    assert!(batch.moves.iter().all(|m| m.from == Engine::Claude && m.to == Engine::Codex && m.model.is_none()));
    let d = run(&[claude(96, 30), codex(10, 10)], &team(Engine::Claude));
    assert_eq!(d.batch.as_ref().map(|b| b.why), Some(Why::Drain));
    assert_eq!(moved(&d), ["backend", "mobile", "frontend", "data", "qa", "designer", "pm"]);
    // Codex running low: data and qa first.
    let d = run(&[claude(10, 10), codex(87, 30)], &team(Engine::Codex));
    assert_eq!(moved(&d), ["data", "qa"]);
    let d = run(&[claude(10, 10), codex(10, 93)], &team(Engine::Codex));
    assert_eq!(moved(&d), ["data", "qa", "backend", "mobile", "frontend", "designer", "pm"]);
}

#[test]
fn the_role_order_holds_for_both_directions_and_the_persons_own_roles() {
    let order = |to| {
        let mut roles = vec!["pm", "designer", "writer", "qa", "data", "frontend", "mobile", "backend"];
        roles.sort_by_key(|r| rank(to, r));
        roles
    };
    assert_eq!(order(Engine::Codex), ["backend", "mobile", "frontend", "data", "qa", "writer", "designer", "pm"]);
    assert_eq!(order(Engine::Claude), ["data", "qa", "backend", "mobile", "frontend", "writer", "designer", "pm"]);
    // Several of a role: by name.
    let mates = vec![mate("backend-2", Engine::Claude), mate("frontend", Engine::Claude), mate("backend", Engine::Claude)];
    assert_eq!(moved(&run(&[claude(88, 10), codex(0, 0)], &mates)), ["backend", "backend-2", "frontend"]);
}

#[test]
fn the_engine_moved_to_needs_thirty_points_more_room() {
    // Claude at 86 has 14 left: Codex needs 44.
    let cases: &[(&str, u8, u8, bool)] = &[
        ("Codex at 56: 44 left", 56, 0, true),
        ("Codex at 57: 43 left", 57, 0, false),
        ("Codex's week at 57", 0, 57, false),
        ("Codex nearly fresh", 3, 20, true),
    ];
    for (what, five, week, moves) in cases {
        let d = run(&[claude(86, 30), codex(*five, *week)], &team(Engine::Claude));
        assert_eq!(d.batch.as_ref().is_some_and(|b| b.why == Why::Shift), *moves, "{what}: {d:?}");
    }
    // A drain at 96 (4 left): Codex at 66 has 34 — enough; at 67, no.
    assert_eq!(run(&[claude(96, 30), codex(66, 0)], &team(Engine::Claude)).batch.map(|b| b.why), Some(Why::Drain));
    assert_ne!(run(&[claude(96, 30), codex(67, 0)], &team(Engine::Claude)).batch.map(|b| b.why), Some(Why::Drain));
}

#[test]
fn both_engines_low_step_down_on_the_same_engine_and_hold_new_tickets() {
    let mut mates = team(Engine::Claude);
    for m in &mut mates {
        m.model = Some(if m.role == "frontend" { "claude-sonnet-5-5".into() } else { "claude-opus-5-5".into() });
    }
    mates[2].effort = Some("high".into());
    let mut memory = Memory::default();
    let r = [claude(90, 40), codex(88, 30)];
    let d = decide_at(&r, &mates, &reach(), Switching::Auto, NOW, &mut memory);
    let batch = d.batch.expect("stepped down");
    assert_eq!(batch.why, Why::StepDown);
    assert!(batch.moves.iter().all(Move::in_place));
    let frontend = batch.moves.iter().find(|m| m.member == "frontend").unwrap();
    assert_eq!((frontend.model.as_deref(), frontend.effort.as_deref()), (Some("claude-sonnet-5-5"), Some("medium")), "already on Sonnet: less thinking");
    let backend = batch.moves.iter().find(|m| m.member == "backend").unwrap();
    assert_eq!(backend.model.as_deref(), Some("sonnet"));
    assert!(d.hold.contains(&Hold { engine: Engine::Claude, until: NOW + 2 * H }));
    // Once per window: not stepped down again, still held.
    let again = decide_at(&r, &mates, &reach(), Switching::Auto, NOW + M, &mut memory);
    assert!(again.batch.is_none() && !again.hold.is_empty());
    // As light as it goes.
    assert_eq!(step_down(Engine::Claude, Some("sonnet"), Some("low")), None);
    assert_eq!(step_down(Engine::Claude, Some("sonnet"), None), Some((Some("sonnet".into()), Some("low".into()))), "the default is medium");
    assert_eq!(step_down(Engine::Codex, Some("gpt-6-astra"), Some("high")), Some((Some("gpt-6.1-sol".into()), Some("high".into()))));
    assert_eq!(step_down(Engine::Codex, Some("gpt-6.1-sol"), Some("low")), Some((Some("gpt-6.1-sol".into()), Some("minimal".into()))));
    assert_eq!(step_down(Engine::Codex, Some("gpt-6.1-sol"), Some("minimal")), None);
}

#[test]
fn a_stale_reading_is_a_floor_moving_away_but_never_toward() {
    // Claude read 2 hours ago at 88% of a week still running: it is at least that now.
    let old_claude = read_at(Engine::Claude, NOW - 2 * H, None, Some((88, 2 * 24 * H)));
    assert!(assess(&old_claude, &Memory::default(), NOW).stale);
    assert_eq!(run(&[old_claude, codex(5, 5)], &team(Engine::Claude)).batch.map(|b| b.why), Some(Why::Shift));
    // Codex read 40 minutes ago: it may have filled since — read it again first.
    let old_codex = read_at(Engine::Codex, NOW - 40 * M, Some((5, 3 * H)), Some((5, 4 * 24 * H)));
    let d = run(&[claude(90, 30), old_codex], &team(Engine::Claude));
    assert!(d.batch.is_none(), "{d:?}");
    assert_eq!(d.refresh, vec![Engine::Codex]);
    // Never read at all: the same.
    let d = run(&[claude(90, 30)], &team(Engine::Claude));
    assert_eq!((d.batch.is_none(), d.refresh.clone()), (true, vec![Engine::Codex]));
    // Not signed in: nothing to read, nowhere to go.
    let only = Reach { ready: vec![Engine::Claude], ..reach() };
    let d = decide_at(&[claude(90, 30)], &team(Engine::Claude), &only, Switching::Auto, NOW, &mut Memory::default());
    assert!(d.batch.is_none() && d.refresh.is_empty());
    // A stale reading's burn is unknown: no projection from it.
    let mut memory = Memory::default();
    memory.observe(&read_at(Engine::Claude, NOW - 2 * H, Some((50, 2 * H)), None));
    memory.observe(&read_at(Engine::Claude, NOW - 90 * M, Some((65, 2 * H)), None));
    assert_eq!(assess(&read_at(Engine::Claude, NOW - 90 * M, Some((65, 2 * H)), None), &memory, NOW).runs_out_at, None);
}

#[test]
fn an_exhausted_engine_waits_for_a_reset_within_the_hour_and_hands_over_past_it() {
    let soon = read_at(Engine::Claude, NOW, Some((100, 50 * M)), Some((60, 3 * 24 * H)));
    let d = run(&[soon, codex(5, 5)], &team(Engine::Claude));
    assert!(d.batch.is_none());
    assert_eq!(d.hold, vec![Hold { engine: Engine::Claude, until: NOW + 50 * M }]);
    let far = read_at(Engine::Claude, NOW, None, Some((100, 2 * 24 * H)));
    let mut memory = Memory { last_batch: Some(NOW - M), ..Memory::default() };
    let d = decide_at(&[far, codex(5, 5)], &team(Engine::Claude), &reach(), Switching::Auto, NOW, &mut memory);
    let batch = d.batch.expect("handed over, batch gap or not");
    assert_eq!(batch.why, Why::Exhausted);
    assert_eq!(moved_of(&batch).last(), Some(&"pm"));
    assert_eq!(d.hold, vec![Hold { engine: Engine::Claude, until: NOW + 2 * 24 * H }]);
}

fn moved_of(b: &Batch) -> Vec<&str> {
    b.moves.iter().map(|m| m.member.as_str()).collect()
}

#[test]
fn what_codex_cannot_do_yet_stays_on_claude() {
    // Today: Codex members neither edit nor have the board's tools.
    let today = Reach { ready: vec![Engine::Claude, Engine::Codex], codex_edits: false, codex_board: false };
    let d = decide_at(&[claude(97, 30), codex(5, 5)], &team(Engine::Claude), &today, Switching::Auto, NOW, &mut Memory::default());
    assert_eq!(moved(&d), ["data", "qa"]);
    // A shift moves implementation roles only — none of which Codex can take yet.
    let d = decide_at(&[claude(86, 30), codex(5, 5)], &team(Engine::Claude), &today, Switching::Auto, NOW, &mut Memory::default());
    assert!(d.batch.is_none());
    // Toward Claude, anyone.
    let d = decide_at(&[claude(5, 5), codex(5, 93)], &team(Engine::Codex), &today, Switching::Auto, NOW, &mut Memory::default());
    assert_eq!(moved(&d).len(), 7);
}

#[test]
fn off_shows_only_and_the_person_is_told_each_level_once() {
    let d = decide_at(&[claude(97, 30), codex(5, 5)], &team(Engine::Claude), &reach(), Switching::Off, NOW, &mut Memory::default());
    assert_eq!(d.assessments[0].level, Level::Drain);
    assert!(d.batch.is_none() && d.notices.is_empty() && d.hold.is_empty());
    let mut memory = Memory::default();
    let mates = team(Engine::Claude);
    let d = decide_at(&[claude(72, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, NOW, &mut memory);
    assert_eq!(d.notices.len(), 1);
    assert_eq!((d.notices[0].level, d.notices[0].used), (Level::Watch, 72));
    assert!(d.batch.is_none(), "watch only tells");
    let d = decide_at(&[claude(74, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, NOW + M, &mut memory);
    assert!(d.notices.is_empty(), "the same level of the same window: not again");
    let d = decide_at(&[claude(86, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, NOW + 2 * M, &mut memory);
    assert_eq!(d.notices.iter().map(|n| n.level).collect::<Vec<_>>(), [Level::Shift]);
    // A new window: told again.
    let next = read_at(Engine::Claude, NOW + 3 * H, Some((72, 5 * H)), None);
    let d = decide_at(&[next], &mates, &reach(), Switching::Suggest, NOW + 3 * H, &mut memory);
    assert_eq!(d.notices.len(), 1);
}

#[test]
fn batches_are_fifteen_minutes_apart() {
    let mut memory = Memory::default();
    let mut mates = team(Engine::Claude);
    let r = [claude(86, 30), codex(5, 5)];
    let first = decide_at(&r, &mates, &reach(), Switching::Auto, NOW, &mut memory);
    assert_eq!(moved(&first), ["backend", "mobile", "frontend"]);
    for m in mates.iter_mut().filter(|m| ["backend", "mobile", "frontend"].contains(&m.name.as_str())) {
        m.engine = Engine::Codex;
        m.switched.push(NOW);
    }
    let r = [claude(96, 30), codex(15, 5)];
    assert!(decide_at(&r, &mates, &reach(), Switching::Auto, NOW + 14 * M, &mut memory).batch.is_none(), "too soon after the last");
    let d = decide_at(&r, &mates, &reach(), Switching::Auto, NOW + 15 * M, &mut memory);
    assert_eq!(moved(&d), ["data", "qa", "designer", "pm"]);
}

#[test]
fn a_member_stays_an_hour_and_moves_at_most_once_in_three() {
    let cases: &[(&str, Vec<i64>, bool)] = &[
        ("never moved", vec![], true),
        ("moved 30 minutes ago", vec![NOW - 30 * M], false),
        ("moved 2 hours ago", vec![NOW - 2 * H], false),
        ("moved 3 hours ago", vec![NOW - 3 * H], true),
        ("moved twice, the last an hour ago", vec![NOW - 5 * H, NOW - H], false),
    ];
    for (what, switched, moves) in cases {
        let mut backend = mate("backend", Engine::Claude);
        backend.switched = switched.clone();
        let d = run(&[claude(88, 30), codex(5, 5)], &[backend]);
        assert_eq!(!moved(&d).is_empty(), *moves, "{what}");
    }
    let pinned = Mate { pinned: true, ..mate("backend", Engine::Claude) };
    let waiting = Mate { waiting: true, ..mate("mobile", Engine::Claude) };
    assert!(run(&[claude(88, 30), codex(5, 5)], &[pinned, waiting]).batch.is_none(), "pinned, or a move waits already");
}

/// A backend that moved to Codex four hours ago, Claude being its home.
fn away() -> Vec<Mate> {
    vec![Mate { engine: Engine::Codex, home: Engine::Claude, home_model: Some("opus".into()), switched: vec![NOW - 4 * H], ..mate("backend", Engine::Claude) }]
}

#[test]
fn members_come_back_once_their_engine_recovered_for_ten_minutes_and_not_on_a_flicker() {
    let mut memory = Memory::default();
    let mates = away();
    let codex_ok = codex(20, 20);
    // 55: not low enough.
    let d = decide_at(&[read_at(Engine::Claude, NOW, Some((55, 4 * H)), Some((40, 3 * 24 * H))), codex_ok], &mates, &reach(), Switching::Auto, NOW, &mut memory);
    assert!(d.batch.is_none());
    // 45 now: not yet ten minutes.
    let low = |at| read_at(Engine::Claude, at, Some((45, 4 * H)), Some((40, 3 * 24 * H)));
    let d = decide_at(&[low(NOW + M), codex_ok], &mates, &reach(), Switching::Auto, NOW + M, &mut memory);
    assert!(d.batch.is_none());
    let d = decide_at(&[low(NOW + 9 * M), codex_ok], &mates, &reach(), Switching::Auto, NOW + 10 * M, &mut memory);
    assert!(d.batch.is_none(), "nine minutes");
    // A flicker back up starts the count over.
    let up = read_at(Engine::Claude, NOW + 10 * M, Some((52, 4 * H)), Some((40, 3 * 24 * H)));
    decide_at(&[up, codex_ok], &mates, &reach(), Switching::Auto, NOW + 10 * M, &mut memory);
    let d = decide_at(&[low(NOW + 12 * M), codex_ok], &mates, &reach(), Switching::Auto, NOW + 12 * M, &mut memory);
    assert!(d.batch.is_none());
    let d = decide_at(&[low(NOW + 22 * M), codex_ok], &mates, &reach(), Switching::Auto, NOW + 22 * M, &mut memory);
    let batch = d.batch.expect("back home");
    assert_eq!((batch.why, batch.engine), (Why::Return, Engine::Claude));
    assert_eq!((batch.moves[0].from, batch.moves[0].to, batch.moves[0].model.as_deref()), (Engine::Codex, Engine::Claude, Some("opus")));
    // A week still at 75: not back.
    let mut memory = Memory::default();
    let week = |at| read_at(Engine::Claude, at, Some((10, 4 * H)), Some((75, 3 * 24 * H)));
    decide_at(&[week(NOW), codex_ok], &mates, &reach(), Switching::Auto, NOW, &mut memory);
    assert!(decide_at(&[week(NOW + 20 * M), codex_ok], &mates, &reach(), Switching::Auto, NOW + 20 * M, &mut memory).batch.is_none());
}

#[test]
fn between_the_way_out_and_the_way_back_there_is_room_so_nobody_flaps() {
    // Moved at 86; Claude settles at 60 — between 50 and 85: nobody moves either way.
    let mut memory = Memory::default();
    let mut mates = team(Engine::Claude);
    let d = decide_at(&[claude(86, 30), codex(10, 10)], &mates, &reach(), Switching::Auto, NOW, &mut memory);
    for m in d.batch.unwrap().moves {
        let mate = mates.iter_mut().find(|x| x.name == m.member).unwrap();
        (mate.engine, mate.switched) = (m.to, vec![NOW]);
    }
    for minutes in [20, 60, 120, 200, 400] {
        let at = NOW + minutes * M;
        let r = [read_at(Engine::Claude, at, Some((60, 4 * H)), Some((40, 3 * 24 * H))), read_at(Engine::Codex, at, Some((30, 3 * H)), Some((20, 4 * 24 * H)))];
        assert!(decide_at(&r, &mates, &reach(), Switching::Auto, at, &mut memory).batch.is_none(), "{minutes} minutes");
    }
    // Within three hours of moving, not back even once recovered.
    let mut memory = Memory::default();
    let r = |at| [read_at(Engine::Claude, at, Some((10, 4 * H)), Some((20, 3 * 24 * H))), codex(30, 20)];
    decide_at(&r(NOW + 2 * H), &mates, &reach(), Switching::Auto, NOW + 2 * H, &mut memory);
    assert!(decide_at(&r(NOW + 2 * H + 20 * M), &mates, &reach(), Switching::Auto, NOW + 2 * H + 20 * M, &mut memory).batch.is_none());
    let d = decide_at(&r(NOW + 3 * H), &mates, &reach(), Switching::Auto, NOW + 3 * H, &mut memory);
    assert_eq!(d.batch.map(|b| b.why), Some(Why::Return));
}

#[test]
fn a_stale_reading_shows_recovery_only_once_its_limits_started_over() {
    let mates = away();
    let codex_ok = codex(20, 20);
    let cases: &[(&str, Option<(u8, i64)>, Option<(u8, i64)>, bool)] = &[
        ("five hours started over 15 minutes ago, week 60", Some((90, -15 * M)), Some((60, 3 * 24 * H)), true),
        ("started over 5 minutes ago", Some((90, -5 * M)), Some((60, 3 * 24 * H)), false),
        ("the week at 70 may be higher now", Some((90, -15 * M)), Some((70, 3 * 24 * H)), false),
        ("five hours at 20 an hour ago may be higher now", Some((20, 2 * H)), Some((10, 3 * 24 * H)), false),
        ("the week started over too", Some((90, -20 * M)), Some((99, -15 * M)), true),
    ];
    for (what, five, week, back) in cases {
        let stale = read_at(Engine::Claude, NOW - 2 * H, *five, *week);
        let d = run(&[stale, codex_ok], &mates);
        assert_eq!(d.batch.is_some_and(|b| b.why == Why::Return), *back, "{what}");
    }
}

#[test]
fn what_the_person_turned_down_isnt_asked_again_until_it_gets_worse_or_starts_over() {
    let mut memory = Memory::default();
    let mates = team(Engine::Claude);
    let d = decide_at(&[claude(86, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, NOW, &mut memory);
    let b = d.batch.unwrap();
    memory.decline(b.engine, b.why, b.resets_at);
    let later = NOW + 20 * M;
    assert!(decide_at(&[claude(88, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, later, &mut memory).batch.is_none());
    let d = decide_at(&[claude(96, 30), codex(5, 5)], &mates, &reach(), Switching::Suggest, later + M, &mut memory);
    assert_eq!(d.batch.map(|b| b.why), Some(Why::Drain), "worse: asked again");
}

#[test]
fn the_setup_puts_writers_on_claude_and_reviewers_on_codex_once_both_are_signed_in() {
    let mates = team(Engine::Claude);
    assert!(wants_setup(&mates, &reach()));
    let moves = setup(&mates, &reach());
    let to_codex: Vec<&str> = moves.iter().filter(|m| m.to == Engine::Codex).map(|m| m.member.as_str()).collect();
    assert_eq!(to_codex, ["data", "qa"]);
    assert!(moves.iter().all(|m| m.effort.as_deref() == Some("medium")));
    assert_eq!(moves.iter().find(|m| m.member == "qa").unwrap().model.as_deref(), Some("gpt-6.1-sol"));
    // Already on its model: nothing for it.
    let mut on_models = team(Engine::Claude);
    for m in &mut on_models {
        m.model = Some(match m.role.as_str() {
            "frontend" => "claude-sonnet-5-5".into(),
            _ => "claude-opus-5-5".into(),
        });
    }
    let moves = setup(&on_models, &reach());
    assert_eq!(moves.iter().map(|m| m.member.as_str()).collect::<Vec<_>>(), ["data", "qa"]);
    // Mixed already, one engine signed in, or nobody to move: no card.
    let mut mixed = team(Engine::Claude);
    mixed[5].engine = Engine::Codex;
    assert!(!wants_setup(&mixed, &reach()));
    assert!(!wants_setup(&mates, &Reach { ready: vec![Engine::Claude], ..reach() }));
    assert!(!wants_setup(&[mate("frontend", Engine::Claude)], &reach()), "nobody would change engine");
    assert!(!wants_setup(&[], &reach()));
    // A role of the person's own keeps its seat.
    assert_eq!(seat("writer"), None);
}

#[test]
fn codex_models_come_from_the_accounts_catalog() {
    let catalog: Vec<String> = ["gpt-6-astra", "gpt-6-sol", "gpt-6.1-sol", "gpt-6-luna"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(codex_model("gpt-6.1-sol", &catalog).as_deref(), Some("gpt-6.1-sol"));
    let older: Vec<String> = ["gpt-6-astra", "gpt-6-sol"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(codex_model("gpt-6.1-sol", &older).as_deref(), Some("gpt-6-sol"), "the family's newest the account has");
    assert_eq!(codex_model("gpt-6.1-sol", &["gpt-5.6-terra".to_owned()]), None, "the engine's own choice");
}
