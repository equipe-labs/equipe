//! The squad's status at a glance ([`crate::status`]): this gathers what only
//! the running loop knows — the breaker, each engine's plan and pace, who is
//! mid-turn, what waits for the person — in one cheap pass, then reads the
//! journal off the loop and works the answer out. An answer is kept for
//! [`crate::status::CACHE_FOR`], so a screen that polls every half minute costs
//! one journal read per squad per that long at most, however many screens ask;
//! pinning a ticket to watch drops it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use equipe_protocol::status::{EngineBudget, SquadStatus, StatusAsk};
use equipe_protocol::{AuthState, Engine};

use super::*;
use crate::importance;
use crate::status::{self, Facts};

/// What was told of what waits on the person in one squad. Kept in the
/// journal (`Journal::waiting_told`): a daemon restart — an update, a forced
/// restart — used to forget it, and the PM was handed the same stale wait
/// again after each one (Golden Boot, 2026-11-25: four reminders about one
/// Google sign-in step in 51 minutes, one per restart).
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct WaitingTold {
    /// The things the person was last told wait on them, by key.
    #[serde(default)]
    keys: HashSet<String>,
    /// When that was.
    #[serde(default)]
    at: Option<u64>,
    /// The things handed to the PM as stale, by key, with when each was last
    /// seen waiting: one gone for [`STALE_FORGET_AFTER`] is news again when
    /// it comes back; a blink (a restart, a resumed ticket) is not.
    #[serde(default)]
    stale: HashMap<String, u64>,
}

/// A stale wait no longer seen this long is forgotten (unix seconds).
pub(super) const STALE_FORGET_AFTER: u64 = 30 * 60;

impl WaitingTold {
    /// One pass of `keys` waiting now at `now`: what no longer waits is
    /// forgotten — the person's line at once, the PM's stale ones once gone
    /// for [`STALE_FORGET_AFTER`]; what still waits is marked seen.
    pub(super) fn seen(&mut self, keys: &HashSet<String>, now: u64) {
        self.keys.retain(|k| keys.contains(k));
        for (k, at) in self.stale.iter_mut() {
            if keys.contains(k) {
                *at = now;
            }
        }
        self.stale.retain(|_, at| now.saturating_sub(*at) < STALE_FORGET_AFTER);
    }

    pub(super) fn stale_keys(&self) -> HashSet<String> {
        self.stale.keys().cloned().collect()
    }
}

/// The screens are told of a change in a squad's status no more often than this.
const TOLD_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// What was last worked out, by squad and the person's offset from UTC.
pub(super) type Cache = Arc<Mutex<HashMap<(String, i32), (Instant, SquadStatus)>>>;

impl Runtime {
    /// Everything about `squad` that only the running daemon knows.
    pub(super) fn status_facts(&mut self, squad: &str) -> Facts {
        let now = now_secs();
        let tickets = self.tickets(squad);
        let parallel = self.journals.get(squad).map(Journal::parallel).unwrap_or_default();
        let disk = self.disk.status.level;
        let (mut members, mut in_use, mut working) = (0, Vec::new(), HashSet::new());
        for s in self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad)) {
            if s.is_member_itself() && s.member.is_some() && !s.is_pm() {
                members += 1;
                if !in_use.contains(&s.engine) {
                    in_use.push(s.engine);
                }
            }
            if let Some(work) = s.work.as_ref()
                && (s.is_working() || s.is_preparing())
                && !Self::is_parked(s)
            {
                working.insert(work.ticket);
            }
        }
        // Each engine's plan as last read, with its week's pace and the squad's pacing.
        let used = |engine: Engine| crate::lanes::plan_used(self.plans.iter().find(|p| p.engine == engine).map(|p| &p.usage));
        let mut engines: Vec<EngineBudget> = self.engine_budgets(squad, now, disk, &parallel);
        // An engine members run on that nobody read yet is listed, unread.
        for engine in &in_use {
            if !engines.iter().any(|e| e.engine == *engine) {
                engines.push(EngineBudget { engine: *engine, plan_type: None, weekly: None, five_hour: None, out_until: None, runs_out_at: None, lasts: false, pacing_at_once: None, stale: false });
            }
        }
        engines.sort_by_key(|e| e.engine != Engine::Claude);
        let runs_max = in_use
            .iter()
            .filter(|e| !engines.iter().any(|b| b.engine == **e && b.out_until.is_some()))
            .map(|e| crate::lanes::engine_limit(used(*e), disk, &parallel) as u32)
            .sum();
        let signed_out: Vec<Engine> = self
            .auth
            .iter()
            .filter(|(_, fix)| matches!(fix.state, AuthState::SignedOut | AuthState::KeychainLocked | AuthState::OutsideLogin { .. }))
            .map(|(engine, _)| *engine)
            .collect();
        // The flow's view: each member with its runs and limit, each engine's lane.
        let guarded = |engine: Engine| !parallel.is_max() && self.pace_of(squad, engine).held;
        let mut crew = Vec::new();
        let mut running: HashMap<Engine, u32> = HashMap::new();
        for (&id, s) in self.sessions.iter().filter(|(_, s)| s.squad.as_deref() == Some(squad)) {
            if s.run_of.is_some() {
                // A run paused while its engine's reserve is held holds no slot.
                if crate::pacing::holds_slot(Self::is_parked(s), guarded(s.engine)) {
                    *running.entry(s.engine).or_default() += 1;
                }
                continue;
            }
            let Some(m) = s.member.as_ref().filter(|_| s.is_member_itself()) else { continue };
            let runs = self.sessions.values().filter(|r| r.run_of == Some(id)).count() as u32;
            crew.push(crate::speed::Crew { name: m.name.clone(), role: m.role.clone(), engine: s.engine, runs, limit: crate::lanes::member_limit(used(s.engine), disk, &parallel) as u32, pm: s.is_pm() });
        }
        let at = |mode| equipe_protocol::lanes::Parallel { mode, ..parallel };
        let lanes: Vec<crate::speed::Lane> = engines
            .iter()
            .map(|b| {
                let engine = b.engine;
                crate::speed::Lane {
                    engine,
                    cap: crate::lanes::engine_limit(used(engine), disk, &parallel) as u32,
                    running: running.get(&engine).copied().unwrap_or(0),
                    reserve: guarded(engine),
                    per_day_bp: self.week_burn(engine),
                    full_cap: crate::lanes::engine_limit(used(engine), disk, &at(equipe_protocol::lanes::ParallelMode::Max)) as u32,
                    balanced_cap: crate::lanes::engine_limit(used(engine), disk, &at(equipe_protocol::lanes::ParallelMode::Auto)) as u32,
                }
            })
            .collect();
        let approvals = self.pending.values().filter(|p| p.squad == squad).count() as u32;
        let decisions = (self.question_sets(squad).len() + self.team_changes(squad).len() + self.routine_proposals(squad).len() + self.accepts(squad).len() + self.secret_asks(squad).len() + self.cli_sign_in_open(squad).len()) as u32;
        let asks = self.waiting_asks(squad);
        let held = self.held_requests(squad, &tickets, &asks);
        Facts {
            squad: squad.to_owned(),
            tickets,
            members,
            breaker: self.turn_ends.breaker(squad),
            engines,
            in_use,
            working,
            runs_max,
            approvals,
            decisions,
            signed_out,
            pm_accepts: self.journals.get(squad).is_none_or(Journal::pm_accepts),
            held,
            asks,
            mac_locked: self.desktop.locked_waiting().iter().filter(|m| m.squad == squad).count() as u32,
            crew,
            lanes,
            single_project: self.projects(squad).len() == 1,
            parallel,
        }
    }

    /// What waits on the person besides tickets, each with the tickets it is
    /// about and whether a member is parked on it ([`crate::importance`]).
    pub(super) fn waiting_asks(&mut self, squad: &str) -> Vec<importance::Ask> {
        use equipe_protocol::importance::WaitKind;
        let line = |text: &str| -> String { text.lines().next().unwrap_or_default().trim().chars().take(100).collect() };
        let mut out = Vec::new();
        // A request holds its member's turn: a lane parked on it.
        for p in self.pending.values().filter(|p| p.squad == squad) {
            let a = &p.approval;
            // One its member didn't wait for holds nobody idle; it waits since it was asked.
            let later = a.ask.as_ref().is_some_and(|k| k.later);
            let since = if later { self.approvals_asked.get(&a.id).copied().unwrap_or(0) } else { 0 };
            out.push(importance::Ask { kind: WaitKind::Permission, label: line(&format!("{}: {} {}", a.member, a.tool, a.action)), tickets: a.ticket.into_iter().collect(), holding: Vec::new(), since, idle_members: u32::from(!later) });
        }
        // Questions don't hold the member up: they go on without the answer.
        for set in self.question_sets(squad).iter() {
            let label = set.questions.first().map(|q| line(&q.question)).unwrap_or_default();
            out.push(importance::Ask { kind: WaitKind::Decision, label, tickets: set.ticket.into_iter().collect(), holding: Vec::new(), since: set.asked_at, idle_members: 0 });
        }
        for c in self.team_changes(squad).iter() {
            out.push(importance::Ask { kind: WaitKind::Decision, label: line(&c.reason), tickets: Vec::new(), holding: c.unblocks.clone(), since: c.proposed_at, idle_members: 0 });
        }
        for r in self.routine_proposals(squad).iter() {
            out.push(importance::Ask { kind: WaitKind::Decision, label: line(&r.routine.title), tickets: Vec::new(), holding: Vec::new(), since: r.proposed_at, idle_members: 0 });
        }
        for c in self.accepts(squad).iter() {
            out.push(importance::Ask { kind: WaitKind::Decision, label: line(&c.summary), tickets: c.tickets.clone(), holding: Vec::new(), since: c.proposed_at, idle_members: 0 });
        }
        // A secret holds its member until typed.
        for a in self.secret_asks(squad).iter() {
            out.push(importance::Ask { kind: WaitKind::SignIn, label: line(&a.why), tickets: a.ticket.into_iter().collect(), holding: Vec::new(), since: a.at, idle_members: 1 });
        }
        // So does a command-line sign-in the person hasn't finished.
        for c in self.cli_sign_in_open(squad) {
            out.push(importance::Ask { kind: WaitKind::SignIn, label: line(&c.why), tickets: c.ticket.into_iter().collect(), holding: Vec::new(), since: c.at, idle_members: 1 });
        }
        out
    }

    /// The person's request each ticket that may wait on them serves — open
    /// work, accepted work not on main, and what `asks` are about.
    fn held_requests(&self, squad: &str, tickets: &[Ticket], asks: &[importance::Ask]) -> HashMap<TicketId, equipe_protocol::importance::HeldRequest> {
        let Some(journal) = self.journals.get(squad) else { return HashMap::new() };
        let mut ids: Vec<TicketId> = tickets.iter().filter(|t| !t.status.is_closed() || t.waits_to_merge()).map(|t| t.id).collect();
        ids.extend(asks.iter().flat_map(|a| a.tickets.iter().copied()));
        ids.sort_unstable();
        ids.dedup();
        journal.requests_of(&ids).into_iter().map(|(t, r)| (t, equipe_protocol::importance::HeldRequest { id: r.id, title: r.title })).collect()
    }

    /// Everything waiting on the person, the most blocking first, each with
    /// the request it holds up ([`importance::waiting_list`]).
    pub(super) fn waiting_now(&mut self, squad: &str) -> Vec<equipe_protocol::importance::Waiting> {
        let tickets = self.tickets(squad);
        let asks = self.waiting_asks(squad);
        let pm_accepts = self.journals.get(squad).is_none_or(Journal::pm_accepts);
        let held = self.held_requests(squad, &tickets, &asks);
        let mut list = importance::waiting_list(&tickets, &asks, pm_accepts);
        importance::hold_requests(&mut list, |t| held.get(&t).cloned());
        list
    }

    /// The PM's note of everything waiting on the person, most blocking
    /// first, the stale marked ([`importance::pm_note`]): `None` when nothing waits.
    pub(super) fn waiting_note(&mut self, squad: &str) -> Option<String> {
        let list = self.waiting_now(squad);
        importance::pm_note(&list, importance::PM_SHOWN, now_secs())
    }

    /// Each minute: when something new waits on the person — at most every
    /// [`importance::TELL_EVERY`] — they are told all that waits, in one
    /// line, in the PM's conversation; and what has waited
    /// [`importance::STALE_AFTER`] is handed to the PM once, to bring up with
    /// them or find a way around.
    pub(super) async fn tell_waiting_on_person(&mut self, squad: &str) {
        let now = now_secs();
        let list = self.waiting_now(squad);
        let keys: HashSet<String> = list.iter().map(importance::key).collect();
        if !self.waiting_told.contains_key(squad) {
            let kept = self.journals.get(squad).and_then(Journal::waiting_told).and_then(|j| serde_json::from_str::<WaitingTold>(&j).ok()).unwrap_or_default();
            self.waiting_told.insert(squad.to_owned(), kept);
        }
        let told = self.waiting_told.entry(squad.to_owned()).or_default();
        let before = told.clone();
        // What no longer waits is forgotten: waiting again, it is news again.
        told.seen(&keys, now);
        let tell = importance::tell_person(&list, &told.keys, told.at, now);
        let stale: Vec<String> = importance::newly_stale(&list, &told.stale_keys(), now).into_iter().map(importance::key).collect();
        let Some(pm) = self.pm_of(squad) else { return self.keep_waiting_told(squad, &before) };
        if tell && let Some(line) = importance::person_line(&list, crate::i18n::core(self.config.lang), now) {
            let told = self.waiting_told.entry(squad.to_owned()).or_default();
            told.keys = keys.clone();
            told.at = Some(now);
            self.publish(Event::Notice { session: pm, text: line, undo: None });
        }
        if !stale.is_empty() {
            let newly: Vec<&equipe_protocol::importance::Waiting> = list.iter().filter(|w| stale.contains(&importance::key(w))).collect();
            if let Some(text) = importance::stale_message(&newly, now) {
                self.waiting_told.entry(squad.to_owned()).or_default().stale.extend(stale.into_iter().map(|k| (k, now)));
                // Kept before the PM is handed it: a restart right after must not hand it again.
                self.keep_waiting_told(squad, &before);
                crate::daemon_log!("equipe: {squad}: {} things have waited on the person for hours; the PM is told", newly.len());
                self.hand_to_pm(squad, pm, text).await;
                return;
            }
        }
        self.keep_waiting_told(squad, &before);
    }

    /// `squad`'s [`WaitingTold`] into its journal when it changed from `before`
    /// (the "last seen" times move every pass; they are kept at most every few minutes).
    fn keep_waiting_told(&mut self, squad: &str, before: &WaitingTold) {
        let Some(now) = self.waiting_told.get(squad).cloned() else { return };
        let moved = now.keys != before.keys || now.at != before.at || now.stale.keys().collect::<HashSet<_>>() != before.stale.keys().collect::<HashSet<_>>();
        let refresh = now.stale.iter().any(|(k, at)| before.stale.get(k).is_some_and(|b| at.saturating_sub(*b) >= STALE_FORGET_AFTER / 3));
        if !moved && !refresh {
            return;
        }
        if let (Some(journal), Ok(json)) = (self.journals.get(squad), serde_json::to_string(&now))
            && let Err(err) = journal.set_waiting_told(&json)
        {
            crate::daemon_log!("equipe: {squad}: what the person was told waits wasn't kept: {err}");
        }
    }

    /// A squad's status: the kept answer if it is fresh, else worked out now
    /// — the facts here, the journal read off the loop — and kept.
    pub(super) fn squad_status(&mut self, ask: StatusAsk, reply: oneshot::Sender<Result<SquadStatus, String>>) {
        let key = (ask.squad.clone(), ask.utc_offset_secs);
        if let Some((at, kept)) = self.status_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
            && at.elapsed() < status::CACHE_FOR
        {
            let _ = reply.send(Ok(kept.clone()));
            return;
        }
        if self.journal(&ask.squad).is_none() {
            let _ = reply.send(Err(format!("no squad called {}", ask.squad)));
            return;
        }
        let facts = self.status_facts(&ask.squad);
        let db = crate::journal::squad_dir(&self.config.home, &ask.squad).join("squad.db");
        let cache = self.status_cache.clone();
        tokio::spawn(async move {
            let worked = tokio::task::spawn_blocking(move || status::read(&db, &facts, now_secs(), ask.utc_offset_secs)).await;
            let answer = match worked {
                Ok(Ok(status)) => {
                    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Instant::now(), status.clone()));
                    Ok(status)
                }
                Ok(Err(why)) => Err(why),
                Err(_) => Err("the look at the squad's status stopped".to_owned()),
            };
            let _ = reply.send(answer);
        });
    }

    /// Something that changes how `squad`'s work goes happened: the kept
    /// answer is dropped, and the screens are told to ask again — at most
    /// once every [`TOLD_EVERY`].
    pub(super) fn status_changed(&mut self, squad: &str) {
        self.status_cache.lock().unwrap_or_else(|e| e.into_inner()).retain(|(s, _), _| s != squad);
        if self.status_told.get(squad).is_some_and(|at| at.elapsed() < TOLD_EVERY) {
            return;
        }
        self.status_told.insert(squad.to_owned(), Instant::now());
        self.publish(Event::StatusChanged { squad: squad.to_owned() });
    }

    /// The person pins a ticket to watch, or lets it go. The next status says so.
    pub(super) fn watch_ticket(&mut self, squad: &str, ticket: TicketId, watch: bool) {
        match self.journal(squad).map(|j| j.set_watched(ticket, watch)) {
            Some(Ok(_)) => self.status_changed(squad),
            Some(Err(err)) => self.squad_error(squad, format!("{err:#}")),
            None => {}
        }
    }
}

#[cfg(test)]
mod waiting_told_tests {
    use super::*;
    use equipe_protocol::importance::{Impact, WaitKind, Waiting};

    fn wait(ticket: TicketId, since: u64) -> Waiting {
        Waiting { kind: WaitKind::Ticket, ticket: Some(ticket), status: None, title: "Add your Google login".into(), since: Some(since), impact: Impact::default(), asks: Vec::new(), request: None }
    }

    /// Golden Boot, 2026-11-25: one Google sign-in step waited on the person;
    /// the daemon restarted four times in 51 minutes (engine updates) and each
    /// restart handed the PM the same stale wait again, so the person was
    /// reminded four times. What was told now survives a restart.
    #[test]
    fn a_stale_wait_told_once_is_not_told_again_after_a_restart() {
        let w = [wait(39, 1_000)];
        let now = 1_000 + importance::STALE_AFTER + 60;
        let keys: HashSet<String> = w.iter().map(importance::key).collect();
        let mut told = WaitingTold::default();
        told.seen(&keys, now);
        let first = importance::newly_stale(&w, &told.stale_keys(), now);
        assert_eq!(first.len(), 1, "told the first time");
        told.stale.insert(importance::key(&w[0]), now);
        // The restart: what was kept is read back.
        let kept: WaitingTold = serde_json::from_str(&serde_json::to_string(&told).unwrap()).unwrap();
        let mut again = kept;
        for later in [now + 300, now + 1_260, now + 3_060] {
            again.seen(&keys, later);
            assert!(importance::newly_stale(&w, &again.stale_keys(), later).is_empty(), "not again at +{}s", later - now);
        }
    }

    /// A wait that blinks out (a restart before the board is read, a ticket
    /// resumed and put down again) is not news; one gone for half an hour is.
    #[test]
    fn a_stale_wait_is_forgotten_only_after_it_is_gone_a_while() {
        let w = [wait(39, 1_000)];
        let keys: HashSet<String> = w.iter().map(importance::key).collect();
        let now = 1_000 + importance::STALE_AFTER + 60;
        let mut told = WaitingTold::default();
        told.stale.insert(importance::key(&w[0]), now);
        told.seen(&HashSet::new(), now + 120);
        told.seen(&keys, now + 180);
        assert!(importance::newly_stale(&w, &told.stale_keys(), now + 180).is_empty(), "a blink is not news");
        told.seen(&HashSet::new(), now + 180 + STALE_FORGET_AFTER);
        assert!(told.stale.is_empty(), "gone half an hour: forgotten");
        assert_eq!(importance::newly_stale(&w, &told.stale_keys(), now + 200 + STALE_FORGET_AFTER).len(), 1, "back again: news again");
    }

    /// A journal written by an older build (or none) reads as nothing told.
    #[test]
    fn an_older_or_missing_record_reads_as_nothing_told() {
        assert_eq!(serde_json::from_str::<WaitingTold>("{}").unwrap(), WaitingTold::default());
        assert!(serde_json::from_str::<WaitingTold>("not json").is_err());
    }
}
