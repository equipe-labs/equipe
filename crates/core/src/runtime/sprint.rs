//! The sprint loop's runtime side (`crate::sprint` has the rules): each
//! minute equipe looks at every squad's loop — a sprint at its end is
//! reviewed (its facts and the product's numbers gathered off the loop, then
//! handed to the PM in one message), a squad with a product live and no
//! sprint gets its first planned, a proposal's small items start on their own
//! where the person lets them, hypotheses whose tickets went live are locked,
//! checked for their events and judged once their window has passed, and the
//! sprint's spend is counted against its budget. The PM's `plan_sprint` puts
//! the next sprint to the person on one card; their tap starts its items.

use std::sync::Arc;

use equipe_protocol::autonomy::{Kind, Level};
use equipe_protocol::decide::{Answer, QuestionSet};
use equipe_protocol::growth::Evidence;
use equipe_protocol::sprint::{
    Ended, Experiment, ExperimentStatus, ItemStatus, Judged, Phase, SetParams, Sprint, SprintParams, SprintView, Verdict,
};
use equipe_protocol::{Priority, TicketDraft, TicketKind};

use super::*;
use crate::sprint::data::{self, ProductData};
use crate::sprint::platform::{self as platform, Platform};
use crate::sprint::{self as rules, Store};

/// What a review or a planning brought back from off the loop.
#[derive(Default)]
pub(super) struct Gathered {
    pub evidence: Vec<Evidence>,
    pub gaps: Vec<String>,
    /// The north star over the sprint's days, and the days before.
    pub north: Option<(f64, Option<f64>)>,
    /// The sprint reviewed; `None`: a planning.
    pub sprint: Option<u32>,
    /// Hypotheses whose window had passed, judged for the review.
    pub judged: Vec<(u32, Judged)>,
    /// Plain lines about the product that aren't numbers to cite (top sources, experiments running).
    pub notes: Vec<String>,
}

/// The sprint's spend is counted again at most this often.
const COUNT_EVERY: u64 = 10 * 60;

impl Runtime {
    fn sprint_dir(&self, squad: &str) -> PathBuf {
        crate::journal::squad_dir(&self.config.home, squad)
    }

    pub(super) fn sprint_store(&self, squad: &str) -> Store {
        rules::load(&self.sprint_dir(squad))
    }

    /// Kept, and every screen of the squad told to read it again.
    fn keep_sprint(&mut self, squad: &str, store: &Store) {
        if let Err(why) = rules::save(&self.sprint_dir(squad), store) {
            crate::daemon_log!("equipe: {squad}: the sprint loop wasn't kept: {why}");
        }
        self.publish(Event::SprintChanged { squad: squad.to_owned() });
    }

    /// Where the squad's product numbers are read: a source registered
    /// in-process, else its PostHog connection, else the equipe platform of a cloud team.
    pub(super) fn sprint_data(&mut self, squad: &str) -> Option<Arc<dyn ProductData>> {
        if let Some(d) = data::registered(squad) {
            return Some(d);
        }
        let (config, _, _) = self.growth_config(squad);
        if let Some(connection) = self.growth_connection(squad, &config).ok().flatten() {
            let exclude = crate::growth::exclusion(&config).unwrap_or_default();
            return Some(Arc::new(data::PostHog { home: self.config.home.clone(), squad: squad.to_owned(), connection, exclude }));
        }
        // A cloud team: its products live on the equipe platform, which has the numbers.
        Platform::discover().map(|p| Arc::new(p) as Arc<dyn ProductData>)
    }

    /// Whether the squad has a product live: a deploy seen, or analytics connected.
    fn sprint_live(&mut self, squad: &str) -> bool {
        let deployed = self.journal(squad).is_some_and(|j| !j.deploys().is_empty());
        deployed
            || data::registered(squad).is_some()
            || crate::metrics::connections(&self.config.home).iter().any(|c| c.squad == squad && c.kind == equipe_protocol::metrics::SourceKind::Posthog)
            || Platform::discover().is_some_and(|p| platform::live_cached(&p))
    }

    /// Whether the loop runs for the squad; why not.
    pub(super) fn sprint_on(&mut self, squad: &str) -> Result<(), String> {
        let settings = self.sprint_store(squad).settings;
        let has_pm = self.pm_of(squad).is_some();
        if settings.on == Some(false) || !has_pm {
            return rules::enabled(&settings, has_pm, false);
        }
        let live = settings.on == Some(true) || self.sprint_live(squad);
        rules::enabled(&settings, has_pm, live)
    }

    /// The review's slot: as set, else the weekly retro routine's.
    fn sprint_slot(&mut self, squad: &str, store: &Store) -> (u8, u8) {
        let retro = self.journal(squad).and_then(|j| j.retro_routine());
        let when = retro.and_then(|id| self.routines(squad).into_iter().find(|r| r.id == id)).map(|r| r.when);
        let retro = match when {
            Some(equipe_protocol::routines::Schedule::Weekly { weekday, hour, .. }) => Some((weekday, hour)),
            _ => None,
        };
        rules::slot(&store.settings, retro)
    }

    /// The roles and names on the team.
    fn sprint_team(&self, squad: &str) -> Vec<String> {
        self.team(squad).into_iter().flat_map(|(_, name, role)| [name, role]).collect()
    }

    /// Each minute, for every squad: its loop looked at.
    pub(super) async fn sprint_tick(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.sprint_look(&squad, now).await;
        }
    }

    /// The squad's loop at `now`: hypotheses gone live, checked or due; a
    /// sprint at its end reviewed; a review the PM let pass closed; a first
    /// sprint planned; small items started on their own; the spend counted.
    pub(super) async fn sprint_look(&mut self, squad: &str, now: u64) {
        let dir = self.sprint_dir(squad);
        let kept = rules::store_path(&dir).exists();
        let on = self.sprint_on(squad).is_ok();
        if !kept && !on {
            return;
        }
        let mut store = self.sprint_store(squad);
        // Locked first and kept, so a review gathered now reads them live.
        if self.sprint_lock(squad, &mut store, now) {
            self.keep_sprint(squad, &store);
        }
        let mut changed = false;
        if on && !self.sprint_gathering.contains(squad) {
            match store.current().map(|s| (s.id, s.phase, s.ends_at, s.closed_at)) {
                Some((id, Phase::Active, Some(end), _)) if end <= now => {
                    self.sprint_gather(squad, Some(id));
                }
                Some((id, Phase::Review, Some(end), _)) if end + rules::PATIENCE <= now => {
                    // The PM let its review pass: closed on its facts alone, and planned anew.
                    if let Some(s) = store.sprint_mut(id) {
                        s.phase = Phase::Closed;
                        s.closed_at = Some(now);
                    }
                    store.planning_asked = None;
                    changed = true;
                    crate::daemon_log!("equipe: {squad}: S-{id} closed without a retro (the PM didn't plan within a day)");
                    if let Some(pm) = self.pm_of(squad) {
                        let shown = say!(self, "news.sprint_closed_quiet", id = id);
                        self.show_news(pm, shown);
                    }
                }
                None if store.proposed().is_none() && store.planning_asked.is_none_or(|at| at + rules::PLAN_AGAIN <= now) => {
                    self.sprint_gather(squad, None);
                }
                _ => {}
            }
            if store.auto_at.is_some_and(|at| at <= now) {
                store.auto_at = None;
                changed = true;
                if self.autonomy_store(squad).policy.level(Kind::Sprint) != Level::Ask {
                    self.sprint_auto_start(squad, &mut store).await;
                }
            }
            changed |= self.sprint_count(squad, &mut store, now);
        }
        if changed {
            self.keep_sprint(squad, &store);
        }
        self.sprint_due(squad, &store, now);
    }

    /// Hypotheses whose tickets went live are locked, a cancelled ticket's
    /// dropped (and its item).
    fn sprint_lock(&mut self, squad: &str, store: &mut Store, now: u64) -> bool {
        if !store.experiments.iter().any(|x| matches!(x.status, ExperimentStatus::Waiting | ExperimentStatus::Live)) {
            return false;
        }
        let deploys = self.journal(squad).map(Journal::deploys).unwrap_or_default();
        let tickets = self.tickets(squad);
        let mut changed = false;
        let mut live = Vec::new();
        for x in store.experiments.iter_mut().filter(|x| x.status == ExperimentStatus::Waiting) {
            let Some(ticket) = x.ticket.and_then(|t| tickets.iter().find(|tk| tk.id == t)) else { continue };
            if ticket.status == TicketStatus::Cancelled {
                x.status = ExperimentStatus::Dropped;
                changed = true;
            } else if let Some(at) = crate::growth::live_at(ticket, &deploys, now) {
                rules::went_live(x, at);
                live.push(x.clone());
                changed = true;
            }
        }
        for item in store.items.iter_mut().filter(|i| i.status == ItemStatus::Started) {
            if item.ticket.and_then(|t| tickets.iter().find(|tk| tk.id == t)).is_some_and(|t| t.status == TicketStatus::Cancelled) {
                item.status = ItemStatus::Dropped;
                changed = true;
            }
        }
        for x in live {
            crate::daemon_log!("equipe: {squad}: H-{} is live; judged after {} days", x.id, x.rule.window_days);
            let line = format!("H-{} \"{}\" is live: its rule is locked; equipe judges it after {} days.", x.id, x.belief, x.rule.window_days);
            let shown = say!(self, "news.sprint_live", id = x.id, belief = x.belief, days = x.rule.window_days);
            self.brief_pm(squad, line, Some(shown));
        }
        changed
    }

    /// The day after going live a hypothesis' events are looked for; once
    /// its window has passed it is judged — both off the loop, unless a
    /// review being gathered took it.
    fn sprint_due(&mut self, squad: &str, store: &Store, now: u64) {
        if !store.experiments.iter().any(|x| rules::check_due(x, now) || rules::judge_due(x, now)) {
            return;
        }
        let data = self.sprint_data(squad);
        for x in store.experiments.iter().filter(|x| rules::check_due(x, now) || rules::judge_due(x, now)) {
            if !self.sprint_busy.insert((squad.to_owned(), x.id)) {
                continue;
            }
            let (internal, squad_name, x, data) = (self.internal.clone(), squad.to_owned(), x.clone(), data.clone());
            let judging = rules::judge_due(&x, now);
            tokio::spawn(async move {
                if judging {
                    let judged = match &data {
                        Some(d) => data::judge(d.as_ref(), &x, now).await,
                        None => unmeasured(&x, now),
                    };
                    let _ = internal.send(Internal::SprintJudged { squad: squad_name, experiment: x.id, judged: Box::new(judged) });
                } else {
                    let arriving = match &data {
                        Some(d) => data::arriving(d.as_ref(), &x).await,
                        None => Err("no product data is connected".into()),
                    };
                    let _ = internal.send(Internal::SprintChecked { squad: squad_name, experiment: x.id, arriving });
                }
            });
        }
    }

    /// A hypothesis' events looked for: none arriving is told to the PM and
    /// the ticket's member, to fix before the window passes.
    pub(super) fn sprint_checked(&mut self, squad: &str, id: u32, arriving: Result<bool, String>) {
        self.sprint_busy.remove(&(squad.to_owned(), id));
        let mut store = self.sprint_store(squad);
        let Some(x) = store.experiment_mut(id) else { return };
        x.checked = true;
        let x = x.clone();
        self.keep_sprint(squad, &store);
        if let Ok(false) = arriving {
            let events = rules::metric_events(&x.metric).join(", ");
            let ticket = x.ticket.map(|t| format!(" (T-{t})")).unwrap_or_default();
            let line = format!("H-{} \"{}\"{ticket} is live but none of its events ({events}) arrived in its first day: its instrumentation is missing or broken — fix it now, or it will be judged inconclusive.", x.id, x.belief);
            crate::daemon_log!("equipe: {squad}: H-{id}: no events in its first day");
            let shown = say!(self, "news.sprint_no_events", id = x.id, belief = x.belief, events = events);
            if let Some(owner) = x.ticket.and_then(|t| self.tickets(squad).iter().find(|tk| tk.id == t).and_then(|tk| tk.owner.clone().or(tk.assignee.clone()))) {
                self.brief_member(squad, &owner, &line, &shown);
            }
            self.brief_pm(squad, line, Some(shown));
        }
    }

    /// A hypothesis judged: kept, told to the person and the PM.
    pub(super) fn sprint_judged(&mut self, squad: &str, id: u32, judged: Judged) {
        self.sprint_judged_told(squad, id, judged, true);
    }

    /// [`Self::sprint_judged`]; `brief`: the PM is told now (not when its review tells it).
    fn sprint_judged_told(&mut self, squad: &str, id: u32, judged: Judged, brief: bool) {
        self.sprint_busy.remove(&(squad.to_owned(), id));
        let mut store = self.sprint_store(squad);
        let Some(x) = store.experiment_mut(id) else { return };
        x.status = ExperimentStatus::Judged;
        x.result = Some(judged.clone());
        let x = x.clone();
        self.keep_sprint(squad, &store);
        let line = rules::experiment_line(&x);
        crate::daemon_log!("equipe: {squad}: {line}");
        let numbers = rules::result_numbers(&judged);
        let shown = match judged.verdict {
            Verdict::Worked => say!(self, "news.sprint_worked", id = id, belief = x.belief, numbers = numbers),
            Verdict::DidNot => say!(self, "news.sprint_did_not", id = id, belief = x.belief, numbers = numbers),
            Verdict::Inconclusive => say!(self, "news.sprint_inconclusive", id = id, belief = x.belief),
        };
        if brief {
            self.brief_pm(squad, format!("{line}. It goes into the sprint's review."), Some(shown));
        } else if let Some(pm) = self.pm_of(squad) {
            self.show_news(pm, shown);
        }
    }

    /// The sprint's tickets' tokens counted again (at most every
    /// [`COUNT_EVERY`]); a budget level reached told once.
    fn sprint_count(&mut self, squad: &str, store: &mut Store, now: u64) -> bool {
        if self.sprint_counted.get(squad).is_some_and(|at| at + COUNT_EVERY > now) {
            return false;
        }
        self.sprint_counted.insert(squad.to_owned(), now);
        let Some((id, started, tickets, budget)) = store.current().filter(|s| s.phase == Phase::Active).map(|s| (s.id, s.started_at.unwrap_or(now), s.tickets.clone(), s.budget_tokens)) else { return false };
        let spent = self.sprint_spent(squad, started, &tickets);
        let told = store.budget_told;
        if let Some(s) = store.sprint_mut(id) {
            s.spent_tokens = spent;
        }
        if let Some(level) = rules::budget_step(told, spent, budget) {
            store.budget_told = level;
            let line = format!("S-{id} has used {} of its {} token budget ({level}%). Nothing more starts on its own; finish what matters most first.", rules::tokens(spent), rules::tokens(budget));
            let shown = say!(self, "news.sprint_budget", id = id, pct = level, spent = rules::tokens(spent), budget = rules::tokens(budget));
            self.brief_pm(squad, line, Some(shown));
        }
        true
    }

    /// Tokens the turns on `tickets` used since `since`.
    fn sprint_spent(&mut self, squad: &str, since: u64, tickets: &[TicketId]) -> u64 {
        let turns = self.journal(squad).and_then(|j| j.turns_since(since).ok()).unwrap_or_default();
        turns.iter().filter(|t| t.ticket.is_some_and(|id| tickets.contains(&id))).map(|t| t.tokens).sum()
    }

    /// Gather a review's (`sprint`) or a planning's numbers off the loop:
    /// the squad's product numbers as the growth loop reads them, and the
    /// north star over the sprint's days beside the days before.
    fn sprint_gather(&mut self, squad: &str, sprint: Option<u32>) {
        let now = now_secs();
        let store = self.sprint_store(squad);
        let (config, mut gaps, declared) = self.growth_config(squad);
        let connection = match self.growth_connection(squad, &config) {
            Ok(c) => c,
            Err(why) => {
                gaps.push(why);
                None
            }
        };
        let asks = match &connection {
            Some(c) if declared => crate::growth::asks(&config, c).unwrap_or_else(|why| {
                gaps.push(format!("{}: {why}", crate::growth::FILE));
                Vec::new()
            }),
            _ => Vec::new(),
        };
        let mut local = self.growth_signals(squad, now);
        local.extend(self.growth_checks(squad, now));
        let data = self.sprint_data(squad);
        // Without PostHog the product's numbers come from the equipe platform: a short glance for the review.
        let glance_from = if connection.is_none() { Platform::discover() } else { None };
        let north = store.north_star.as_ref().and_then(|n| n.metric.clone());
        let days = i64::from(rules::length(&store.settings));
        let (home, squad_name, internal, client) = (self.config.home.clone(), squad.to_owned(), self.internal.clone(), self.client.clone());
        let health = config.health.clone();
        // Hypotheses due by now are judged with the review, so it tells their verdicts.
        let due: Vec<Experiment> = if sprint.is_some() { store.experiments.iter().filter(|x| rules::judge_due(x, now)).cloned().collect() } else { Vec::new() };
        let due: Vec<Experiment> = due.into_iter().filter(|x| self.sprint_busy.insert((squad.to_owned(), x.id))).collect();
        self.sprint_gathering.insert(squad.to_owned());
        crate::daemon_log!("equipe: {squad}: the sprint loop gathers for {}", sprint.map_or_else(|| "a planning".to_owned(), |s| format!("S-{s}'s review")));
        tokio::spawn(async move {
            let growth = super::growth::gather(&home, &squad_name, connection.as_deref(), &asks, &health, &client, now).await;
            let mut out = Gathered { evidence: growth.evidence, gaps, north: None, sprint, judged: Vec::new(), notes: Vec::new() };
            if let Some(p) = &glance_from {
                match p.snapshot(7).await {
                    Ok(snap) => {
                        let (evidence, notes) = platform::glance(&snap);
                        out.evidence.extend(evidence);
                        out.notes = notes;
                    }
                    Err(why) => out.gaps.push(why),
                }
            }
            for x in &due {
                let judged = match &data {
                    Some(d) => data::judge(d.as_ref(), x, now).await,
                    None => unmeasured(x, now),
                };
                out.judged.push((x.id, judged));
            }
            out.evidence.extend(local);
            out.gaps.extend(growth.gaps);
            if let (Some(metric), Some(data)) = (north, data) {
                let today = crate::metrics::day_of(&routines::Local, now);
                let (to, from) = (today - 1, today - days);
                match data.read(&metric, from, to).await {
                    Ok(now) => out.north = Some((now.value, data.read(&metric, from - days, from - 1).await.ok().map(|r| r.value))),
                    Err(why) => out.gaps.push(format!("the north star: {why}")),
                }
            }
            let _ = internal.send(Internal::SprintGathered { squad: squad_name, gathered: Box::new(out) });
        });
    }

    /// A review's or a planning's numbers back: handed to the PM in one message.
    pub(super) async fn sprint_gathered(&mut self, squad: &str, g: Gathered) {
        self.sprint_gathering.remove(squad);
        let now = now_secs();
        for (id, judged) in g.judged {
            self.sprint_judged_told(squad, id, judged, false);
        }
        let Some(pm) = self.pm_of(squad) else { return };
        let mut store = self.sprint_store(squad);
        let mut evidence = g.evidence;
        evidence.truncate(rules::EVIDENCE);
        for (n, e) in evidence.iter_mut().enumerate() {
            e.id = format!("E{}", n + 1);
        }
        store.evidence = evidence;
        if let (Some(n), Some((value, previous))) = (store.north_star.as_mut(), g.north) {
            n.value = Some(value);
            n.previous = previous;
        }
        let source = self.sprint_data(squad).map(|d| d.source());
        let measurable = source.is_some();
        let platform = source.as_deref() == Some("equipe platform");
        let notes = g.notes;
        let marketer = self.team(squad).iter().any(|(_, _, role)| role == crate::roles::MARKETING);
        let later: Vec<String> = store.items.iter().filter(|i| i.status == ItemStatus::Later).map(|i| format!("P-{} {} — {}", i.id, i.title, i.expected)).collect();
        let board = self.tickets(squad);
        let today = crate::metrics::day_string(crate::metrics::day_of(&routines::Local, now));
        let reviewed = g.sprint.and_then(|id| store.sprints.iter().find(|s| s.id == id && s.phase == Phase::Active).cloned());
        let message = match reviewed {
            Some(sprint) => {
                // Tried before: the hypotheses of earlier sprints, not this one's (listed on their own).
                let earlier: Vec<Experiment> = store.experiments.iter().filter(|x| !sprint.experiments.contains(&x.id)).cloned().collect();
                let tried = rules::tried(&earlier);
                let from = sprint.started_at.unwrap_or(sprint.proposed_at);
                let week = self.sprint_week(squad, from, now);
                let spent = self.sprint_spent(squad, from, &sprint.tickets);
                let facts = rules::facts(&sprint, &board, &week, &store.experiments, spent);
                let tickets: Vec<String> = sprint.tickets.iter().filter_map(|id| board.iter().find(|t| t.id == *id)).map(rules::ticket_line).collect();
                let experiments: Vec<String> = store.experiments.iter().filter(|x| sprint.experiments.contains(&x.id) || (x.status == ExperimentStatus::Live)).map(rules::experiment_line).collect();
                let troubles: Vec<String> = week.troubles.iter().take(6).map(|t| format!("{} · {}{}", match t.snag {
                    crate::retro::Snag::Failed => "failed",
                    crate::retro::Snag::SentBack => "sent back",
                    crate::retro::Snag::Stuck => "stuck",
                    crate::retro::Snag::Late => "late",
                }, t.title.clone().or_else(|| t.ticket.map(|id| format!("T-{id}"))).map(|s| format!("{s}: ")).unwrap_or_default(), t.why)).collect();
                let friction: Vec<String> = week.friction.iter().take(5).map(|f| format!("{} ({}): {} snag(s), waited on the person {}×", f.member, f.role, f.snags.values().sum::<usize>(), f.waits)).collect();
                let review = rules::Review {
                    sprint: Some(&sprint),
                    from: crate::metrics::day_string(crate::metrics::day_of(&routines::Local, from)),
                    to: today.clone(),
                    tickets,
                    experiments,
                    tried,
                    north_star: store.north_star.as_ref(),
                    facts: Some(&facts),
                    budget: sprint.budget_tokens,
                    troubles,
                    friction,
                    evidence: &store.evidence,
                    gaps: g.gaps.clone(),
                    market: store.market,
                    marketer,
                    measurable,
                    later,
                    platform,
                    notes,
                };
                let mut text = rules::review_message(&review);
                if let Some(line) = self.journal(squad).and_then(|j| j.turn_endings_since(from).ok()).filter(|c| !c.is_empty()) {
                    let listed: Vec<String> = line.iter().map(|(k, n)| format!("{} {n}", k.replace('_', " "))).collect();
                    text.push_str(&format!("\nTurns that didn't end as they should: {}.", listed.join(", ")));
                }
                if let Some(s) = store.sprint_mut(sprint.id) {
                    s.phase = Phase::Review;
                    s.ends_at = Some(now);
                    s.spent_tokens = spent;
                    s.facts = Some(facts);
                }
                crate::daemon_log!("equipe: {squad}: S-{} reviewed; handed to the PM", sprint.id);
                self.show_news(pm, say!(self, "news.sprint_review", id = sprint.id));
                text
            }
            None => {
                let tried = rules::tried(&store.experiments);
                let review = rules::Review { to: today, tried, north_star: store.north_star.as_ref(), evidence: &store.evidence, gaps: g.gaps.clone(), market: store.market, marketer, measurable, later, platform, notes, ..rules::Review::default() };
                let text = rules::planning_message(&review);
                store.planning_asked = Some(now);
                crate::daemon_log!("equipe: {squad}: the PM is asked to plan a sprint");
                self.show_news(pm, say!(self, "news.sprint_planning"));
                text
            }
        };
        store.count_turn(now, crate::growth::estimate(&message));
        self.keep_sprint(squad, &store);
        match self.sessions.get(&pm) {
            Some(s) if s.is_working() => self.leave_memo(pm, message),
            _ => self.submit(pm, message).await,
        }
    }

    /// The retro's facts of `from..to`, from the squad's journal.
    fn sprint_week(&mut self, squad: &str, from: u64, to: u64) -> crate::retro::Facts {
        let Some(journal) = self.journal(squad) else { return crate::retro::Facts::default() };
        let tickets = journal.tickets().unwrap_or_default();
        let events = journal.ticket_events_since(from.saturating_sub(crate::retro::WEEK)).unwrap_or_default();
        let turns = journal.turns_since(from).unwrap_or_default();
        let waits = journal.waits_since(from).unwrap_or_default();
        crate::retro::gather(&crate::retro::Week { from, to, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs: Vec::new() })
    }

    /// `plan_sprint`: the PM's proposal checked ([`rules::plan`]), the sprint
    /// in review closed with its retro, the items ranked and put to the
    /// person on one card — those that may start on their own started now
    /// or after the window, as the person's autonomy setting says.
    pub(super) async fn sprint_plan(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM plans sprints; tell the PM what you would put in the next one.".into());
        }
        let now = now_secs();
        let mut store = self.sprint_store(squad);
        let projects: Vec<String> = self.projects(squad).iter().map(|p| board::project_name(p)).collect();
        let team = self.sprint_team(squad);
        let reviewing = store.current().filter(|s| s.phase == Phase::Review).map(|s| s.id);
        let plan = rules::plan(args, &rules::PlanCtx { store: &store, projects: &projects, team: &team, reviewing, now })?;
        let board = self.tickets(squad);
        let mut carried = Vec::new();
        if let Some(rid) = reviewing
            && let Some(s) = store.sprint_mut(rid)
        {
            s.phase = Phase::Closed;
            s.closed_at = Some(now);
            s.retro = plan.retro.clone();
            carried = s.tickets.iter().copied().filter(|t| board.iter().any(|tk| tk.id == *t && !tk.status.is_closed())).collect();
        }
        if let Some(n) = plan.north_star.clone() {
            let kept = store.north_star.take();
            store.north_star = Some(match kept {
                Some(k) if k.metric == n.metric => equipe_protocol::sprint::NorthStar { value: k.value, previous: k.previous, ..n },
                _ => n,
            });
        }
        // Items the person left for later and the PM didn't propose again are let go.
        for i in store.items.iter_mut().filter(|i| i.status == ItemStatus::Later) {
            i.status = ItemStatus::Dropped;
        }
        let budget = rules::budget(&store.settings);
        let mut items = plan.items;
        rules::rank(&mut items, budget);
        let sid = store.next_sprint + 1;
        let sprint = Sprint {
            id: sid,
            goal: plan.goal,
            done_when: plan.done_when,
            phase: Phase::Proposed,
            proposed_at: now,
            started_at: None,
            ends_at: None,
            closed_at: None,
            items: items.iter().map(|i| i.id).collect(),
            tickets: carried,
            experiments: Vec::new(),
            budget_tokens: budget,
            spent_tokens: 0,
            set: 0,
            retro: None,
            facts: None,
        };
        store.next_sprint = sid;
        store.next_item = items.iter().map(|i| i.id).max().unwrap_or(store.next_item);
        store.items.extend(items.iter().cloned());
        store.sprints.push(sprint);
        store.planning_asked = None;
        // The person's autonomy setting: small items start now, after the window, or only on their tap.
        let policy = self.autonomy_store(squad).policy;
        let mut started = Vec::new();
        match policy.level(Kind::Sprint) {
            Level::JustDo => started = self.sprint_auto_start(squad, &mut store).await,
            Level::Tell if items.iter().any(|i| i.recommended && i.may_auto_start) => store.auto_at = Some(now + policy.window_secs()),
            _ => {}
        }
        let ask_market = store.market.is_none();
        let Some(proposed) = store.sprints.iter().find(|s| s.id == sid).cloned() else { return Err("The sprint wasn't kept.".into()) };
        let shown: Vec<equipe_protocol::sprint::Item> = store.items.iter().filter(|i| proposed.items.contains(&i.id)).cloned().collect();
        let questions = rules::questions(&proposed, &shown.iter().collect::<Vec<_>>(), ask_market, plan.market_guess, self.config.lang);
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session: id, member: member.to_owned(), ticket: None, questions, asked_at: now, review: false, retro: None };
        if let Some(s) = store.sprint_mut(sid) {
            s.set = set.id;
        }
        let ranked: Vec<String> = shown.iter().map(|i| format!("P-{} {}{}", i.id, i.title, if i.recommended { " (recommended)" } else { "" })).collect();
        let count = shown.len();
        self.keep_sprint(squad, &store);
        equipe_protocol::decide::add(self.question_sets(squad), set.clone());
        self.save_questions(squad);
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: set });
        crate::daemon_log!("equipe: {squad}: S-{sid} proposed with {count} items");
        self.show_news(id, say!(self, "news.sprint_proposed", id = sid, count = count));
        let closed = reviewing.map(|r| format!("S-{r} is closed with your retro. ")).unwrap_or_default();
        let auto = if started.is_empty() { String::new() } else { format!(" Started on their own under the person's autonomy setting: {}.", started.join("; ")) };
        Ok(format!(
            "{closed}Proposed S-{sid}, ranked by impact against cost: {}. The person starts it with one tap on its card; their answer comes as \
your next message.{auto} Don't make these items' tickets yourself.",
            ranked.join("; ")
        ))
    }

    /// The proposal's recommended small low-risk items started on their own,
    /// while the sprint has budget: their lines.
    async fn sprint_auto_start(&mut self, squad: &str, store: &mut Store) -> Vec<String> {
        let Some(p) = store.proposed().or(store.current().filter(|s| s.phase == Phase::Active)).cloned() else { return Vec::new() };
        let mine: Vec<equipe_protocol::sprint::Item> = store.items.iter().filter(|i| p.items.contains(&i.id)).cloned().collect();
        let ids: Vec<u32> = rules::auto_starting(&mine, p.spent_tokens, p.budget_tokens).iter().map(|i| i.id).collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let lines = self.sprint_start(squad, store, p.id, &ids, true);
        if !lines.is_empty() {
            let shown = say!(self, "news.sprint_auto_started", id = p.id, items = lines.join("; "));
            if let Some(pm) = self.pm_of(squad) {
                self.show_news(pm, shown);
            }
            self.dispatch(squad).await;
        }
        lines
    }

    /// Start `ids` of sprint `sid`: a ticket each (with its hypothesis
    /// registered), the sprint under way if it wasn't; a ticket for the data
    /// analyst to check the new hypotheses' metrics before they ship. Lines
    /// of what was made.
    fn sprint_start(&mut self, squad: &str, store: &mut Store, sid: u32, ids: &[u32], auto: bool) -> Vec<String> {
        let now = now_secs();
        let slot = self.sprint_slot(squad, store);
        let length = rules::length(&store.settings);
        if let Some(s) = store.sprint_mut(sid)
            && s.phase == Phase::Proposed
        {
            s.phase = Phase::Active;
            s.started_at = Some(now);
            s.ends_at = Some(rules::ends_at(now, slot, length, &routines::Local));
            store.budget_told = 0;
        }
        let team = self.team(squad);
        let projects = self.projects(squad);
        let mut lines = Vec::new();
        let mut new_experiments = Vec::new();
        for id in ids {
            let Some(item) = store.item(*id).filter(|i| i.status == ItemStatus::Proposed).cloned() else { continue };
            let experiment = item.hypothesis.as_ref().map(|h| rules::experiment(store.next_experiment + 1, sid, h, None, Some(item.id), "pm", now));
            let (title, body) = rules::ticket_draft(&item, experiment.as_ref());
            let assignee = item.owner.clone().filter(|o| team.iter().any(|(_, name, role)| name == o || role == o));
            let project = item.project.as_ref().and_then(|p| projects.iter().find(|path| &board::project_name(path) == p).cloned());
            let kind = match item.kind {
                equipe_protocol::sprint::ItemKind::Marketing if project.is_none() => Some(TicketKind::Task),
                _ => None,
            };
            let draft = TicketDraft { title, body, project, assignee, priority: Priority::default(), depends_on: Vec::new(), kind };
            match self.new_ticket(squad, draft) {
                Ok(ticket) => {
                    if let Some(mut x) = experiment {
                        x.ticket = Some(ticket);
                        store.next_experiment = x.id;
                        new_experiments.push(x.id);
                        if let Some(s) = store.sprint_mut(sid) {
                            s.experiments.push(x.id);
                        }
                        if let Some(i) = store.item_mut(item.id) {
                            i.experiment = Some(x.id);
                        }
                        store.experiments.push(x);
                    }
                    if let Some(s) = store.sprint_mut(sid) {
                        s.tickets.push(ticket);
                    }
                    if let Some(i) = store.item_mut(item.id) {
                        i.status = ItemStatus::Started;
                        i.ticket = Some(ticket);
                        i.auto_started = auto;
                    }
                    lines.push(format!("P-{} {} → T-{ticket}", item.id, item.title));
                }
                Err(why) => lines.push(format!("P-{} {}: its ticket couldn't be made ({why})", item.id, item.title)),
            }
        }
        // The data analyst checks the new hypotheses' metrics and rules before they ship.
        if !new_experiments.is_empty()
            && let Some((_, data_member, _)) = team.iter().find(|(_, _, role)| role == "data")
        {
            let listed: Vec<String> = store.experiments.iter().filter(|x| new_experiments.contains(&x.id)).map(rules::experiment_line).collect();
            let draft = TicketDraft {
                title: format!("S-{sid}: check the success metrics before they ship"),
                body: format!(
                    "Each hypothesis below is judged by its rule once its ticket has been live for its window. Before each ships, check that its \
metric counts what the change should move, that the rule's change, days and fewest counted are right for the product's traffic, and \
that its events are (or will be) sent; refine it with `{}` (it locks once its ticket is live). Report what you changed in a line.\n\n{}",
                    rules::DEFINE,
                    listed.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
                ),
                project: None,
                assignee: Some(data_member.clone()),
                priority: Priority::default(),
                depends_on: Vec::new(),
                kind: Some(TicketKind::Task),
            };
            if let Ok(ticket) = self.new_ticket(squad, draft)
                && let Some(s) = store.sprint_mut(sid)
            {
                s.tickets.push(ticket);
            }
        }
        lines
    }

    /// The person answered (or closed) a sprint's card: the items they
    /// picked started, the rest left for later; "not now" starts none. The
    /// message for the PM in place of the plain answers; `None`: not a sprint's.
    pub(super) async fn sprint_settled(&mut self, squad: &str, set: &QuestionSet, answers: Option<&[Answer]>) -> Option<String> {
        let mut store = self.sprint_store(squad);
        let sprint = store.sprints.iter().find(|s| s.set == set.id && matches!(s.phase, Phase::Proposed | Phase::Active)).cloned()?;
        let shown: Vec<equipe_protocol::sprint::Item> = store.items.iter().filter(|i| sprint.items.contains(&i.id)).cloned().collect();
        let refs: Vec<&equipe_protocol::sprint::Item> = shown.iter().collect();
        let ask_market = set.questions.len() > 2;
        let picked = rules::picked(answers, &refs, ask_market);
        if let Some(m) = picked.market {
            store.market = Some(m);
        }
        store.auto_at = None;
        let now = now_secs();
        let message = if picked.not_now && sprint.phase == Phase::Proposed {
            if let Some(s) = store.sprint_mut(sprint.id) {
                s.phase = Phase::Declined;
            }
            for i in store.items.iter_mut().filter(|i| sprint.items.contains(&i.id) && i.status == ItemStatus::Proposed) {
                i.status = ItemStatus::Later;
            }
            store.planning_asked = Some(now);
            crate::daemon_log!("equipe: {squad}: the person said not now to S-{}", sprint.id);
            format!("The person said not now to S-{}: nothing of it started; its items wait as \"later\". equipe asks you to plan again in about a week, or sooner if they ask.", sprint.id)
        } else {
            let wanted: Vec<u32> = picked.items.iter().copied().filter(|i| shown.iter().any(|s| s.id == *i && s.status == ItemStatus::Proposed)).collect();
            let lines = self.sprint_start(squad, &mut store, sprint.id, &wanted, false);
            for i in store.items.iter_mut().filter(|i| sprint.items.contains(&i.id) && i.status == ItemStatus::Proposed) {
                i.status = ItemStatus::Later;
            }
            let end = store.sprints.iter().find(|s| s.id == sprint.id).and_then(|s| s.ends_at).map(|e| crate::metrics::day_string(crate::metrics::day_of(&routines::Local, e))).unwrap_or_default();
            crate::daemon_log!("equipe: {squad}: S-{} started with {} item(s)", sprint.id, lines.len());
            if let Some(pm) = self.pm_of(squad) {
                self.show_news(pm, say!(self, "news.sprint_started", id = sprint.id, count = lines.len(), end = end.clone()));
            }
            format!(
                "The person started S-{} (reviewed {end}):\n{}\nThe rest wait as \"later\". Give each new ticket without an owner one; a hypothesis' \
events go in with its change.",
                sprint.id,
                if lines.is_empty() { "- nothing new (its items had started already)".to_owned() } else { lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n") }
            )
        };
        self.keep_sprint(squad, &store);
        self.dispatch(squad).await;
        let market = picked.market.map(|m| format!("\nThe person says the product is {}.", match m {
            equipe_protocol::sprint::Market::Consumers => "sold to people (B2C): plan go-to-market work with the marketer",
            equipe_protocol::sprint::Market::Businesses => "sold to businesses (B2B): plan go-to-market work with the marketer",
            equipe_protocol::sprint::Market::Internal => "not sold (an internal tool): no go-to-market work",
        })).unwrap_or_default();
        Some(format!("{message}{market}"))
    }

    /// `define_experiment`: a ticket's hypothesis set or refined, before it is live.
    pub(super) fn define_experiment(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let role = self.sessions.get(&id).and_then(|s| s.member.as_ref()).map(|m| m.role.clone()).unwrap_or_default();
        if role != crate::roles::PM && role != "data" {
            return Err("Only the PM and the data analyst define hypotheses; tell them what should be measured.".into());
        }
        let ticket = args.get("ticket").and_then(Value::as_u64).ok_or("Which `ticket` ships the change?")? as TicketId;
        let tickets = self.tickets(squad);
        let t = tickets.iter().find(|t| t.id == ticket).ok_or_else(|| format!("There is no T-{ticket}."))?;
        if t.status.is_closed() {
            return Err(format!("T-{ticket} is {} already.", t.status.label()));
        }
        let mut store = self.sprint_store(squad);
        let h = rules::hypothesis_of(args, &store.evidence)?;
        let now = now_secs();
        let line = match store.experiments.iter_mut().find(|x| x.ticket == Some(ticket) && x.status != ExperimentStatus::Dropped) {
            Some(x) if x.status != ExperimentStatus::Waiting => {
                return Err(format!("H-{}'s rule is locked: T-{ticket} went live, and it is judged by the rule it had then.", x.id));
            }
            Some(x) => {
                x.belief = h.belief;
                x.metric = h.metric;
                x.rule = h.rule;
                x.split = h.split;
                x.events = h.events;
                x.defined_by = member.to_owned();
                x.defined_at = now;
                format!("Refined H-{}: {}", x.id, rules::experiment_line(x))
            }
            None => {
                let sprint = store.current().or(store.proposed()).map_or(0, |s| s.id);
                let x = rules::experiment(store.next_experiment + 1, sprint, &h, Some(ticket), None, member, now);
                store.next_experiment = x.id;
                if let Some(s) = store.sprint_mut(sprint) {
                    s.experiments.push(x.id);
                }
                let line = format!("Registered H-{} for T-{ticket}: {}", x.id, rules::experiment_line(&x));
                store.experiments.push(x);
                line
            }
        };
        let data = self.sprint_data(squad);
        let split = store.experiments.iter().find(|x| x.ticket == Some(ticket)).is_some_and(|x| x.split.is_some());
        self.keep_sprint(squad, &store);
        let warn = match &data {
            None => " No product data is connected: it will be judged inconclusive unless analytics are connected before its window passes.",
            Some(d) if split && !d.splits() => " The product data has no feature flags: it will be read before beside after, not A/B.",
            _ => "",
        };
        Ok(format!("{line}.{warn} It locks when T-{ticket} goes live."))
    }

    /// A screen's `sprint.*` request: its answer, or a JSON-RPC error.
    pub(super) fn sprint_request(&mut self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        let squad = params.get("squad").and_then(Value::as_str).unwrap_or_default().to_owned();
        if !crate::squads::valid_name(&squad) || !crate::squads::exists(&self.config.home, &squad) || self.journal(&squad).is_none() {
            return Err((-32602, format!("there is no squad {squad}")));
        }
        let answer = match method {
            equipe_protocol::sprint::GET => {
                let _: SprintParams = serde_json::from_value(params).map_err(|e| (-32602, format!("{method}: {e}")))?;
                serde_json::to_value(self.sprint_view(&squad))
            }
            equipe_protocol::sprint::END => serde_json::to_value(self.sprint_end(&squad)),
            equipe_protocol::sprint::SET => {
                let p: SetParams = serde_json::from_value(params).map_err(|e| (-32602, format!("{method}: {e}")))?;
                self.sprint_set(&p).map_err(|why| (-32602, why))?;
                serde_json::to_value(self.sprint_view(&squad))
            }
            _ => return Err((-32601, "unknown method".into())),
        };
        answer.map_err(|e| (-32000, e.to_string()))
    }

    /// The person ends the sprint under way now (or asks for a first one).
    fn sprint_end(&mut self, squad: &str) -> Ended {
        if let Err(why) = self.sprint_on(squad) {
            return Ended { started: false, why: Some(why) };
        }
        if self.sprint_gathering.contains(squad) {
            return Ended { started: false, why: Some("a review is being gathered".into()) };
        }
        let mut store = self.sprint_store(squad);
        if self.sprint_lock(squad, &mut store, now_secs()) {
            self.keep_sprint(squad, &store);
        }
        match store.current().map(|s| (s.id, s.phase)) {
            Some((id, Phase::Active)) => self.sprint_gather(squad, Some(id)),
            Some((id, _)) => return Ended { started: false, why: Some(format!("S-{id} is in review already: the PM plans the next one")) },
            None if store.proposed().is_some() => return Ended { started: false, why: Some("the next sprint waits on your answer on its card".into()) },
            None => self.sprint_gather(squad, None),
        }
        Ended { started: true, why: None }
    }

    /// The person's settings, checked and kept.
    fn sprint_set(&mut self, p: &SetParams) -> Result<(), String> {
        let mut store = self.sprint_store(&p.squad);
        let s = &mut store.settings;
        if p.auto {
            s.on = None;
        } else if p.on.is_some() {
            s.on = p.on;
        }
        if let Some(d) = p.weekday {
            if !(1..=7).contains(&d) {
                return Err("a weekday is 1 (Monday) to 7 (Sunday)".into());
            }
            s.weekday = Some(d);
        }
        if let Some(h) = p.hour {
            if h > 23 {
                return Err("an hour is 0 to 23".into());
            }
            s.hour = Some(h);
        }
        if let Some(l) = p.length_days {
            if l != 7 && l != 14 {
                return Err("a sprint runs 7 or 14 days".into());
            }
            s.length_days = l;
        }
        if let Some(b) = p.budget_tokens {
            s.budget_tokens = b;
        }
        let budget = rules::budget(&store.settings);
        if let Some(c) = store.current_mut().filter(|c| c.phase == Phase::Active) {
            c.budget_tokens = budget;
        }
        let squad = p.squad.clone();
        crate::daemon_log!("equipe: {squad}: the sprint loop's settings changed");
        self.keep_sprint(&squad, &store);
        Ok(())
    }

    /// The squad's loop as screens show it.
    pub(super) fn sprint_view(&mut self, squad: &str) -> SprintView {
        let store = self.sprint_store(squad);
        let on = self.sprint_on(squad);
        let (weekday, hour) = self.sprint_slot(squad, &store);
        let board = self.tickets(squad);
        let current = store.current().cloned();
        let current_done = current.as_ref().map_or(0, |c| rules::done(c, &board));
        let last = store.last_closed().cloned();
        let proposed = store.proposed().cloned();
        let shown: Vec<u32> = [&current, &last, &proposed].iter().filter_map(|s| s.as_ref()).flat_map(|s| s.items.clone()).collect();
        let sprints: Vec<u32> = [&current, &last, &proposed].iter().filter_map(|s| s.as_ref()).map(|s| s.id).collect();
        let items = store.items.iter().filter(|i| shown.contains(&i.id)).cloned().collect();
        let mut experiments: Vec<Experiment> = store.experiments.iter().filter(|x| sprints.contains(&x.sprint) || x.status == ExperimentStatus::Live).cloned().collect();
        experiments.sort_by_key(|x| std::cmp::Reverse(x.id));
        let next_review = match &current {
            Some(c) if c.phase == Phase::Active => c.ends_at,
            _ => None,
        };
        let measurable = self.sprint_data(squad).is_some();
        SprintView {
            on: on.is_ok(),
            why_off: on.err(),
            weekday,
            hour,
            length_days: rules::length(&store.settings),
            budget_tokens: rules::budget(&store.settings),
            settings: store.settings.clone(),
            north_star: store.north_star.clone(),
            market: store.market,
            current,
            current_done,
            last,
            proposed,
            items,
            experiments,
            next_review,
            measurable,
        }
    }

    /// The sprint in a few lines for the PM's turns ([`rules::context`]).
    pub(super) fn sprint_context(&mut self, squad: &str) -> Option<String> {
        let path = rules::store_path(&self.sprint_dir(squad));
        if !path.exists() {
            return None;
        }
        let store = self.sprint_store(squad);
        let board = self.tickets(squad);
        let done = store.current().map_or(0, |c| rules::done(c, &board));
        rules::context(&store, done, now_secs())
    }
}

/// A hypothesis due with no product data to read it by.
fn unmeasured(x: &Experiment, now: u64) -> Judged {
    let ((_, _), (af, at)) = crate::growth::windows(x.live_at.unwrap_or(now), x.rule.window_days);
    Judged {
        verdict: Verdict::Inconclusive,
        why: "no product data is connected to read its numbers".into(),
        worse: false,
        before: None,
        after: None,
        arms: Vec::new(),
        change_pct: None,
        from: crate::metrics::day_string(af),
        to: crate::metrics::day_string(at),
        before_from: String::new(),
        before_to: String::new(),
        query: String::new(),
        source: String::new(),
        at: now,
    }
}
