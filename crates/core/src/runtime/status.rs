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
        let approvals = self.pending.values().filter(|p| p.squad == squad).count() as u32;
        let decisions = (self.question_sets(squad).len() + self.team_changes(squad).len() + self.routine_proposals(squad).len() + self.accepts(squad).len() + self.secret_asks(squad).len()) as u32;
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
            asks: self.waiting_asks(squad),
            mac_locked: self.desktop.locked_waiting().iter().filter(|m| m.squad == squad).count() as u32,
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
            out.push(importance::Ask { kind: WaitKind::Permission, label: line(&format!("{}: {} {}", a.member, a.tool, a.action)), tickets: a.ticket.into_iter().collect(), holding: Vec::new(), since: 0, idle_members: 1 });
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
        out
    }

    /// The PM's note of what waits on the person, most blocking first
    /// ([`importance::pm_note`]): `None` when nothing waits that holds work up.
    pub(super) fn waiting_note(&mut self, squad: &str) -> Option<String> {
        let tickets = self.tickets(squad);
        let asks = self.waiting_asks(squad);
        let pm_accepts = self.journals.get(squad).is_none_or(Journal::pm_accepts);
        importance::pm_note(&importance::waiting_list(&tickets, &asks, pm_accepts), 3, now_secs())
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
