//! Why the squad isn't faster, and what another speed would do to the
//! budget (the Status screen's "Why not faster?" and Speed cards).
//!
//! Two pure functions over what the daemon knows:
//!
//! - [`analyze`] sorts the board's open work by what holds it: runs going,
//!   tickets that start in a moment, ready tickets held by the caps, ready
//!   tickets nobody on the squad takes, work waiting for an engine, work
//!   waiting for other tickets, and work waiting for the person — directly,
//!   or behind a ticket that does. The largest of the things that hold work
//!   back is the headline ([`Limit`]).
//! - [`outlook`] says when each engine's week would run out at full speed
//!   and when balanced: the week's pace of late, scaled by how many runs
//!   each speed would have going, never more than the work there is to do.
//!
//! The ready tickets are placed the way the lane policy places them
//! ([`crate::lanes::assign`]): in the board's order, each to the member of
//! its role with the fewest runs, while the member and its engine have room.

use std::collections::{HashMap, HashSet};

use equipe_protocol::lanes::Parallel;
use equipe_protocol::status::{EngineBudget, Flow, Limit, Outlook, SpeedOutlook};
use equipe_protocol::{Engine, PauseWhy, Ticket, TicketId, TicketStatus};

use crate::{board, pacing};

const DAY: i64 = 24 * 60 * 60;

/// A member as the flow sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crew {
    pub name: String,
    pub role: String,
    pub engine: Engine,
    /// Its runs at work now.
    pub runs: u32,
    /// How many it may have at once.
    pub limit: u32,
    /// The squad's PM: admitted even where the reserve is held.
    pub pm: bool,
}

/// An engine's lane as the flow sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub engine: Engine,
    /// How many runs it may have at once across the squad, now.
    pub cap: u32,
    /// Runs at work on it now.
    pub running: u32,
    /// Its reserve is held: only the PM's and urgent or high work start.
    pub reserve: bool,
    /// The week's burn at the pace of late, in basis points of the plan per day.
    pub per_day_bp: Option<u32>,
    /// How many runs it would have at once at full speed, and balanced.
    pub full_cap: u32,
    pub balanced_cap: u32,
}

/// What [`analyze`] reads.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub tickets: &'a [Ticket],
    pub crew: &'a [Crew],
    pub lanes: &'a [Lane],
    pub engines: &'a [EngineBudget],
    /// Tickets whose run is mid-turn now.
    pub working: &'a HashSet<TicketId>,
    pub single_project: bool,
    pub pm_accepts: bool,
}

/// The flow, with the ready work each engine could take: what [`outlook`]
/// reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    pub flow: Flow,
    /// Ready tickets that would run on each engine, started or held by a cap.
    pub demand: HashMap<Engine, u32>,
}

/// Whether the person holds a ticket up: it waits on them, or on a decision
/// that comes to them (an idea, a proposal), or on a sign-in only they can do.
pub fn waits_on_person(t: &Ticket, pm_accepts: bool) -> bool {
    match t.status {
        TicketStatus::Waiting | TicketStatus::Proposed | TicketStatus::Idea => true,
        TicketStatus::InProgress => t.execution.as_ref().is_some_and(|e| e.state == equipe_protocol::ExecutionState::Stalled || e.pause.as_ref().is_some_and(|p| p.why == PauseWhy::SignIn)),
        TicketStatus::Review => !pm_accepts && t.ready_for_review(),
        _ => false,
    }
}

/// Sort the open work by what holds it.
pub fn analyze(input: &Input<'_>) -> Analysis {
    let Input { tickets, crew, lanes, engines, working, single_project, pm_accepts } = *input;
    let by_id: HashMap<TicketId, &Ticket> = tickets.iter().map(|t| (t.id, t)).collect();
    let finished = |id: &TicketId| by_id.get(id).is_some_and(|t| t.status.is_closed());
    let mut flow = Flow { running: working.len() as u32, ..Flow::default() };

    // Who holds work up: tickets on the person, and what waits behind them.
    let person: HashSet<TicketId> = tickets.iter().filter(|t| !t.status.is_closed() && waits_on_person(t, pm_accepts)).map(|t| t.id).collect();
    flow.on_you = person.len() as u32;
    let behind = |t: &Ticket| {
        // Walk the dependencies that aren't finished, looking for one the person holds.
        let (mut stack, mut seen) = (t.depends_on.clone(), HashSet::new());
        while let Some(id) = stack.pop() {
            if finished(&id) || !seen.insert(id) {
                continue;
            }
            if person.contains(&id) {
                return true;
            }
            stack.extend(by_id.get(&id).map(|d| d.depends_on.clone()).unwrap_or_default());
        }
        false
    };

    // Runs paused for the engine (not for a sign-in: that is the person's).
    flow.on_engine = tickets.iter().filter(|t| !t.status.is_closed()).filter(|t| t.execution.as_ref().and_then(|e| e.pause.as_ref()).is_some_and(|p| p.why != PauseWhy::SignIn)).count() as u32;

    // Todo tickets: waiting for others, or ready.
    let mut ready: Vec<&Ticket> = Vec::new();
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Todo) {
        if t.depends_on.iter().all(finished) {
            ready.push(t);
        } else if behind(t) {
            flow.behind_you += 1;
        } else {
            flow.on_tickets += 1;
        }
    }

    // Place the ready ones in the board's order.
    let out = |engine: Engine| engines.iter().any(|e| e.engine == engine && e.out_until.is_some());
    let mut runs: Vec<u32> = crew.iter().map(|m| m.runs).collect();
    let mut room: HashMap<Engine, u32> = lanes.iter().map(|l| (l.engine, l.cap.saturating_sub(l.running))).collect();
    let reserve: HashSet<Engine> = lanes.iter().filter(|l| l.reserve).map(|l| l.engine).collect();
    let mut demand: HashMap<Engine, u32> = HashMap::new();
    let ready_ids: HashSet<TicketId> = ready.iter().map(|t| t.id).collect();
    for t in board::ready_in_order(tickets, &[]).into_iter().filter(|t| ready_ids.contains(&t.id)) {
        let Some(owner) = t.owner.as_deref().filter(|_| board::knows_where(t, single_project)) else {
            flow.no_member += 1;
            continue;
        };
        // A member that has left hands its tickets to its role.
        let owner = if crew.iter().any(|m| m.role == owner || m.name == owner) { owner } else { board::role_of(owner) };
        // An owner that is a role goes to the members of that role; else to the member of that name.
        let by_role = crew.iter().any(|m| m.role == owner);
        let takers: Vec<usize> = (0..crew.len()).filter(|&i| if by_role { crew[i].role == owner } else { crew[i].name == owner }).collect();
        if takers.is_empty() {
            flow.no_member += 1;
            continue;
        }
        // Members whose engine takes this ticket now.
        let usable: Vec<usize> = takers.iter().copied().filter(|&i| !out(crew[i].engine) && (!reserve.contains(&crew[i].engine) || crew[i].pm || pacing::admitted(t.priority))).collect();
        let Some(first) = usable.first() else {
            flow.on_engine += 1;
            continue;
        };
        *demand.entry(crew[*first].engine).or_default() += 1;
        // The member with the fewest runs that has room, on an engine with room.
        let pick = usable.iter().copied().filter(|&i| runs[i] < crew[i].limit && room.get(&crew[i].engine).copied().unwrap_or(0) > 0).min_by_key(|&i| runs[i]);
        match pick {
            Some(i) => {
                runs[i] += 1;
                if let Some(r) = room.get_mut(&crew[i].engine) {
                    *r -= 1;
                }
                flow.starting += 1;
            }
            None => flow.held_by_cap += 1,
        }
    }
    for lane in lanes {
        *demand.entry(lane.engine).or_default() += lane.running;
    }
    flow.limit = limit_of(&flow);
    Analysis { flow, demand }
}

/// What holds the work back most: the largest of the counts, a tie going to
/// what the person or equipe can change first (the cap, then the person).
fn limit_of(flow: &Flow) -> Limit {
    let held = [
        (Limit::Cap, flow.held_by_cap),
        (Limit::You, flow.on_you + flow.behind_you),
        (Limit::Engine, flow.on_engine),
        (Limit::NoMember, flow.no_member),
        (Limit::Tickets, flow.on_tickets),
    ];
    let mut top = (Limit::Flowing, 0);
    for (limit, count) in held {
        if count > top.1 {
            top = (limit, count);
        }
    }
    match top {
        (_, 0) if flow.running == 0 && flow.starting == 0 => Limit::Idle,
        (limit, _) => limit,
    }
}

/// When an engine's week runs out at `scale` times its pace of late, or that
/// it lasts until it starts over.
fn at_scale(budget: &EngineBudget, per_day_bp: u32, scale: f64, now: u64) -> Option<Outlook> {
    let week = budget.weekly?;
    let per_sec = f64::from(per_day_bp) / DAY as f64 * scale;
    if per_sec <= 0.0 {
        return None;
    }
    let left = f64::from(100u32.saturating_sub(u32::from(week.used_percent.min(100)))) * 100.0;
    let at = now as i64 + (left / per_sec).round() as i64;
    Some(if at < week.resets_at { Outlook { runs_out_at: Some(at.max(0) as u64), lasts: false } } else { Outlook { runs_out_at: None, lasts: true } })
}

/// Each engine's week at full speed and balanced. The speed in use is what
/// the pace says already (`budget.runs_out_at`, `lasts`); the other is that
/// pace scaled by how many runs it would have going against now — the cap
/// or the work there is, whichever is less. With nothing running there is
/// no pace to scale, and the other speed is left unsaid.
pub fn outlook(analysis: &Analysis, lanes: &[Lane], engines: &[EngineBudget], parallel: &Parallel, now: u64) -> Vec<SpeedOutlook> {
    engines
        .iter()
        .filter(|b| b.out_until.is_none() && b.weekly.is_some())
        .filter_map(|b| {
            let lane = lanes.iter().find(|l| l.engine == b.engine)?;
            let demand = analysis.demand.get(&b.engine).copied().unwrap_or(0);
            let eff = |cap: u32| f64::from(cap.min(demand));
            let current = eff(lane.cap);
            let now_outlook = if b.runs_out_at.is_some() || b.lasts { Some(Outlook { runs_out_at: b.runs_out_at, lasts: b.lasts && b.runs_out_at.is_none() }) } else { None };
            let other = |cap: u32| -> Option<Outlook> {
                let per_day = lane.per_day_bp?;
                if current <= 0.0 {
                    return None;
                }
                at_scale(b, per_day, eff(cap) / current, now)
            };
            let (full, balanced) = if parallel.is_max() { (now_outlook, other(lane.balanced_cap)) } else { (other(lane.full_cap), now_outlook) };
            Some(SpeedOutlook { engine: b.engine, full, balanced })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::{Priority, TicketKind};

    fn ticket(id: TicketId, status: TicketStatus, deps: &[TicketId]) -> Ticket {
        Ticket {
            id,
            title: format!("T{id}"),
            body: String::new(),
            status,
            kind: TicketKind::Task,
            owner: Some("frontend".into()),
            assignee: None,
            project: None,
            depends_on: deps.to_vec(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: vec![],
            images: vec![],
            priority: Priority::Normal,
            rank: 0,
            updated_at: None,
            design_qa: None,
            plan: None,
            model: None,
        }
    }

    fn member(name: &str, engine: Engine, runs: u32, limit: u32) -> Crew {
        Crew { name: name.into(), role: equipe_role(name), engine, runs, limit, pm: false }
    }

    fn equipe_role(name: &str) -> String {
        board::role_of(name).to_owned()
    }

    fn lane(engine: Engine, cap: u32, running: u32) -> Lane {
        Lane { engine, cap, running, reserve: false, per_day_bp: None, full_cap: 16, balanced_cap: 8 }
    }

    fn run(tickets: &[Ticket], crew: &[Crew], lanes: &[Lane], working: &[TicketId]) -> Analysis {
        let working: HashSet<TicketId> = working.iter().copied().collect();
        analyze(&Input { tickets, crew, lanes, engines: &[], working: &working, single_project: true, pm_accepts: true })
    }

    #[test]
    fn the_real_squad_one_ready_ticket_while_the_rest_waits_on_tickets_and_on_the_person() {
        // 5 runs; 1 ready; 14 tickets waiting for the person; 46 waiting on dependencies, 31 of them behind those 14.
        let mut tickets: Vec<Ticket> = (1..=5).map(|id| ticket(id, TicketStatus::InProgress, &[])).collect();
        tickets.push(ticket(6, TicketStatus::Todo, &[]));
        tickets.extend((100..114).map(|id| ticket(id, TicketStatus::Waiting, &[])));
        // 31 behind the person: half directly, half through another waiting ticket.
        tickets.extend((200..216).map(|id| ticket(id, TicketStatus::Todo, &[100 + id % 14])));
        tickets.extend((300..315).map(|id| ticket(id, TicketStatus::Todo, &[200 + id % 16])));
        // 15 more wait on tickets still in progress.
        tickets.extend((400..415).map(|id| ticket(id, TicketStatus::Todo, &[1 + id % 5])));
        let crew = [member("frontend", Engine::Claude, 5, 6)];
        let lanes = [lane(Engine::Claude, 16, 5)];
        let a = run(&tickets, &crew, &lanes, &[1, 2, 3, 4, 5]);
        let f = &a.flow;
        assert_eq!((f.running, f.starting, f.held_by_cap), (5, 1, 0));
        assert_eq!((f.on_you, f.behind_you, f.on_tickets), (14, 31, 15));
        assert_eq!(f.limit, Limit::You, "the person's 14 hold up 31 more: that is the headline, not the cap");
    }

    #[test]
    fn ready_tickets_over_the_cap_are_held_by_it_and_the_cap_is_the_headline() {
        let mut tickets: Vec<Ticket> = (1..=3).map(|id| ticket(id, TicketStatus::InProgress, &[])).collect();
        tickets.extend((10..16).map(|id| ticket(id, TicketStatus::Todo, &[])));
        let crew = [member("frontend", Engine::Claude, 3, 3)];
        let a = run(&tickets, &crew, &[lane(Engine::Claude, 8, 3)], &[1, 2, 3]);
        assert_eq!((a.flow.starting, a.flow.held_by_cap, a.flow.limit), (0, 6, Limit::Cap));
        // A second member with room takes some of them; the engine's own cap bounds the rest.
        let crew = [member("frontend", Engine::Claude, 3, 3), member("frontend-2", Engine::Claude, 0, 3)];
        let a = run(&tickets, &crew, &[lane(Engine::Claude, 5, 3)], &[1, 2, 3]);
        assert_eq!((a.flow.starting, a.flow.held_by_cap), (2, 4), "the engine's cap: 5 runs, 3 going");
    }

    #[test]
    fn a_ready_ticket_with_nobody_on_its_role_has_no_member() {
        let mut t = ticket(1, TicketStatus::Todo, &[]);
        t.owner = Some("designer".into());
        let a = run(&[t], &[member("frontend", Engine::Claude, 0, 3)], &[lane(Engine::Claude, 8, 0)], &[]);
        assert_eq!((a.flow.no_member, a.flow.starting, a.flow.limit), (1, 0, Limit::NoMember));
        // No owner at all: nobody to take it either.
        let mut unowned = ticket(2, TicketStatus::Todo, &[]);
        unowned.owner = None;
        assert_eq!(run(&[unowned], &[member("frontend", Engine::Claude, 0, 3)], &[lane(Engine::Claude, 8, 0)], &[]).flow.no_member, 1);
    }

    #[test]
    fn work_for_an_engine_out_of_its_limit_or_kept_in_reserve_waits_on_the_engine() {
        let tickets = [ticket(1, TicketStatus::Todo, &[])];
        let crew = [member("frontend", Engine::Codex, 0, 3)];
        let out = EngineBudget { engine: Engine::Codex, plan_type: None, weekly: None, five_hour: None, out_until: Some(9), runs_out_at: None, lasts: false, pacing_at_once: None, stale: false };
        let working = HashSet::new();
        let a = analyze(&Input { tickets: &tickets, crew: &crew, lanes: &[lane(Engine::Codex, 8, 0)], engines: &[out], working: &working, single_project: true, pm_accepts: true });
        assert_eq!((a.flow.on_engine, a.flow.starting, a.flow.limit), (1, 0, Limit::Engine));
        // Reserve: a normal ticket waits, an urgent one and the PM's go.
        let held = Lane { reserve: true, ..lane(Engine::Codex, 8, 0) };
        let a = analyze(&Input { tickets: &tickets, crew: &crew, lanes: std::slice::from_ref(&held), engines: &[], working: &working, single_project: true, pm_accepts: true });
        assert_eq!((a.flow.on_engine, a.flow.starting), (1, 0));
        let mut urgent = ticket(2, TicketStatus::Todo, &[]);
        urgent.priority = Priority::Urgent;
        let a = analyze(&Input { tickets: &[urgent], crew: &crew, lanes: &[held], engines: &[], working: &working, single_project: true, pm_accepts: true });
        assert_eq!((a.flow.on_engine, a.flow.starting), (0, 1));
        // Another engine's member takes the work: nothing waits on the engine.
        let both = [member("frontend", Engine::Codex, 0, 3), member("frontend-2", Engine::Claude, 0, 3)];
        let a = analyze(&Input { tickets: &tickets, crew: &both, lanes: &[lane(Engine::Codex, 8, 0), lane(Engine::Claude, 8, 0)], engines: &[EngineBudget { out_until: Some(9), ..engine_budget(Engine::Codex) }], working: &working, single_project: true, pm_accepts: true });
        assert_eq!((a.flow.on_engine, a.flow.starting), (0, 1));
    }

    fn engine_budget(engine: Engine) -> EngineBudget {
        EngineBudget { engine, plan_type: None, weekly: None, five_hour: None, out_until: None, runs_out_at: None, lasts: false, pacing_at_once: None, stale: false }
    }

    #[test]
    fn runs_paused_for_the_engine_count_there_and_a_paused_sign_in_counts_as_the_persons() {
        use equipe_protocol::{ExecutionState, RunPause, TicketExecution};
        let paused = |id, why| {
            let mut t = ticket(id, TicketStatus::InProgress, &[]);
            t.execution = Some(Box::new(TicketExecution {
                state: ExecutionState::Active,
                session: 1,
                operation: "run".into(),
                generation: 1,
                heartbeat_at: 0,
                lease_expires_at: 0,
                reason: None,
                pause: Some(RunPause { why, engine: Some(Engine::Claude), resumes_at: None, moves_to: None, cause: None, attempt: 0 }),
                head: None,
                dirty: None,
            }));
            t
        };
        let tickets = [paused(1, PauseWhy::EngineLimit), paused(2, PauseWhy::Retrying), paused(3, PauseWhy::SignIn)];
        let a = run(&tickets, &[member("frontend", Engine::Claude, 0, 3)], &[lane(Engine::Claude, 8, 0)], &[]);
        assert_eq!((a.flow.on_engine, a.flow.on_you), (2, 1));
    }

    #[test]
    fn nothing_open_is_idle_and_work_with_nothing_held_is_flowing() {
        let a = run(&[ticket(1, TicketStatus::Done, &[])], &[member("frontend", Engine::Claude, 0, 3)], &[lane(Engine::Claude, 8, 0)], &[]);
        assert_eq!(a.flow.limit, Limit::Idle);
        let a = run(&[ticket(1, TicketStatus::InProgress, &[])], &[member("frontend", Engine::Claude, 1, 3)], &[lane(Engine::Claude, 8, 1)], &[1]);
        assert_eq!(a.flow.limit, Limit::Flowing);
    }

    #[test]
    fn a_tie_goes_to_what_the_person_or_equipe_can_change_and_waiting_on_tickets_is_last() {
        let flow = Flow { held_by_cap: 3, on_you: 2, behind_you: 1, ..Flow::default() };
        assert_eq!(limit_of(&flow), Limit::Cap);
        let flow = Flow { on_you: 2, on_tickets: 3, running: 1, ..Flow::default() };
        assert_eq!(limit_of(&flow), Limit::Tickets, "more waits on tickets than on the person");
        let flow = Flow { on_you: 3, on_tickets: 3, running: 1, ..Flow::default() };
        assert_eq!(limit_of(&flow), Limit::You);
    }

    #[test]
    fn a_dependency_cycle_through_the_person_terminates() {
        let tickets = [ticket(1, TicketStatus::Todo, &[2]), ticket(2, TicketStatus::Todo, &[1]), ticket(3, TicketStatus::Waiting, &[])];
        let a = run(&tickets, &[], &[], &[]);
        assert_eq!((a.flow.on_tickets, a.flow.behind_you, a.flow.on_you), (2, 0, 1));
    }

    fn week(used: u8, resets_in: i64, now: u64) -> Option<equipe_protocol::PlanWindow> {
        Some(equipe_protocol::PlanWindow { used_percent: used, resets_at: now as i64 + resets_in })
    }

    #[test]
    fn full_speed_runs_the_week_out_sooner_when_there_is_work_to_fill_it_and_the_same_when_there_is_not() {
        const NOW: u64 = 1_800_000_000;
        // 40% used, 3 days left, burning 10 points a day at 4 runs; the week would last 6 days: out in 6 days, so before the reset.
        let budget = EngineBudget { weekly: week(40, 3 * DAY, NOW), runs_out_at: None, lasts: true, ..engine_budget(Engine::Claude) };
        let lanes = [Lane { per_day_bp: Some(1_000), full_cap: 16, balanced_cap: 4, ..lane(Engine::Claude, 4, 4) }];
        // Plenty of ready work: full speed would run 8 at once, twice the pace: out in 3 days.
        let busy = Analysis { demand: [(Engine::Claude, 12)].into(), ..Analysis::default() };
        let out = outlook(&busy, &lanes, std::slice::from_ref(&budget), &Parallel::default(), NOW);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].balanced, Some(Outlook { runs_out_at: None, lasts: true }), "balanced is the pace as it is");
        // 12 runs of demand against 4 going: 3x the pace, 60 points left at 30 a day: out in 2 days, before the reset in 3.
        let full = out[0].full.unwrap();
        assert_eq!((full.lasts, full.runs_out_at), (false, Some(NOW + 2 * DAY as u64)));
        // Only 4 runs of work in all: full speed is no faster than balanced.
        let thin = Analysis { demand: [(Engine::Claude, 4)].into(), ..Analysis::default() };
        let out = outlook(&thin, &lanes, std::slice::from_ref(&budget), &Parallel::default(), NOW);
        assert_eq!(out[0].full, Some(Outlook { runs_out_at: None, lasts: true }));
        // On full speed it is the other way round: the pace is full's, balanced is estimated.
        let at_max = EngineBudget { runs_out_at: Some(NOW + 2 * DAY as u64), lasts: false, ..budget.clone() };
        let lanes_max = [Lane { per_day_bp: Some(3_000), full_cap: 12, balanced_cap: 4, ..lane(Engine::Claude, 12, 12) }];
        let out = outlook(&busy, &lanes_max, &[at_max], &Parallel::max(6, 16), NOW);
        assert_eq!(out[0].full, Some(Outlook { runs_out_at: Some(NOW + 2 * DAY as u64), lasts: false }));
        assert_eq!(out[0].balanced, Some(Outlook { runs_out_at: None, lasts: true }), "a third of the pace: 60 points left at 10 a day outlasts the 3 days");
        // Nothing running, nothing to scale: the other speed is left unsaid. An engine out is not listed.
        let idle = Analysis::default();
        let lanes_idle = [Lane { per_day_bp: Some(1_000), ..lane(Engine::Claude, 4, 0) }];
        assert_eq!(outlook(&idle, &lanes_idle, std::slice::from_ref(&budget), &Parallel::default(), NOW)[0].full, None);
        assert!(outlook(&busy, &lanes, &[EngineBudget { out_until: Some(1), ..budget }], &Parallel::default(), NOW).is_empty());
    }
}
