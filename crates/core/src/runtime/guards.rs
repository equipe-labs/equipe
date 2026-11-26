//! The waste stops (`crate::spend`) and the squad's self-watch
//! (`crate::health`), carried out: a ticket that cost too much stops before
//! its next turn and goes to the PM; a squad spending far faster than usual,
//! or every plan nearly out, is told; and every [`health::EVERY`] the squad
//! looks at itself, telling what it finds once (docs/HEALTH.md). Git is read
//! off the loop; everything else here is a few journal queries.

use super::*;
use crate::health::{self, Finding, Live, RepoFacts, Snapshot};
use crate::spend::{self, Card, Trip};

/// Turns a ticket's repeat count remembers, at most.
const TURNS_KEPT: usize = 32;

/// A ticket's turns, oldest first: the member that took each, and the commit its branch was at after.
type Turns = Vec<(String, Option<String>)>;

#[derive(Default)]
pub(super) struct Guards {
    /// Each ticket's turns that ended with its work not done, since it last
    /// stopped: who took each, and the commit its branch was at after.
    turns: HashMap<(String, TicketId), Turns>,
    /// The PM's card for a ticket stopping, until its hand-back is written.
    cards: HashMap<(String, TicketId), Card>,
    rate: HashMap<String, spend::RateState>,
    /// The highest plan headroom level told, per squad.
    headroom: HashMap<String, u8>,
    watched_at: Option<std::time::Instant>,
    /// Each squad's projects as git last had them, and the squads being looked at.
    repos: HashMap<String, Vec<RepoFacts>>,
    looking: HashSet<String>,
}

impl Runtime {
    /// After a turn of run `id` on `ticket` that ended with its work not
    /// done, where equipe would carry it on by itself (the member has the
    /// board's tools, nothing waits on the person, the person said nothing
    /// meanwhile): kept for the repeat count, then — when the ticket cost too
    /// much — it stops here, before another turn, and goes to the PM. Whether it did.
    pub(super) fn budget_stop(&mut self, id: SessionId, squad: &str, name: &str, ticket: TicketId) -> bool {
        let Some(s) = self.sessions.get(&id) else { return false };
        // Where its branch is: read from git's files, no git run.
        let tip = s
            .work
            .as_ref()
            .and_then(|w| w.sandbox.as_ref())
            .and_then(|sb| worktree::git_dirs(&sb.repo).and_then(|(_, common)| health::branch_tip(&common, &sb.branch)));
        let key = (squad.to_owned(), ticket);
        let turns = self.guards.turns.entry(key.clone()).or_default();
        turns.push((name.to_owned(), tip));
        if turns.len() > TURNS_KEPT {
            turns.remove(0);
        }
        let Some(card) = self.spend_trip(squad, ticket, name) else { return false };
        crate::daemon_log!("equipe: {squad}: T-{ticket} stopped before another turn: {}", card.why());
        self.guards.turns.remove(&key);
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
            work.end.handed_back = Some(turn_end::HandBack { reason: turn_end::Reason::Budget, tickets: Vec::new(), suggest: None, note: card.record_line(), blocked_on: None });
        }
        self.guards.cards.insert(key, card);
        self.end_work(id, WorkEnd::HandedBack);
        true
    }

    /// The PM's card for `ticket` stopping, kept by [`Runtime::budget_stop`].
    pub(super) fn take_budget_card(&mut self, squad: &str, ticket: TicketId) -> Option<Card> {
        self.guards.cards.remove(&(squad.to_owned(), ticket))
    }

    /// Whether `ticket`, whose last turn `member` took, stops now — and the PM's card if so.
    fn spend_trip(&mut self, squad: &str, ticket: TicketId, member: &str) -> Option<Card> {
        let repeated = spend::repeats(self.guards.turns.get(&(squad.to_owned(), ticket)).map(Vec::as_slice).unwrap_or(&[]));
        let journal = self.journal(squad)?;
        let found = journal.ticket(ticket).ok().flatten()?;
        let settings = spend::for_ticket(journal.spend_settings(), &found.title, &found.body);
        let (turns, spent) = journal.ticket_spend(&[ticket]).ok()?.get(&ticket).copied().unwrap_or((0, 0));
        // Under the least it could stop at, and not repeating: no need to read its record.
        let over = settings.ticket_budget > 0 && spent >= settings.ticket_budget;
        let repeating = settings.repeat_turns > 0 && repeated >= settings.repeat_turns;
        if !over && !repeating {
            return None;
        }
        let record = journal.ticket_entries(ticket).ok()?;
        let trip = spend::trip(&settings, spent, spend::limit(&settings, &record), repeated, member)?;
        Some(Card::new(&found, &record, spent, turns, trip, &settings))
    }

    /// Before work is handed out: a ticket to do whose turns cost its budget
    /// already (sent back, passed on, put back) doesn't start; it goes to the PM.
    pub(super) fn hold_over_budget(&mut self, squad: &str, tickets: &mut Vec<Ticket>) {
        let Some(journal) = self.journal(squad) else { return };
        let squad_settings = journal.spend_settings();
        // Each ticket's own guards: a design setup step has its hard budget even where the squad set none.
        let own = |t: &Ticket| spend::for_ticket(squad_settings, &t.title, &t.body);
        let todo: Vec<TicketId> = tickets.iter().filter(|t| t.status == TicketStatus::Todo && own(t).ticket_budget > 0).map(|t| t.id).collect();
        if todo.is_empty() {
            return;
        }
        let spent = journal.ticket_spend(&todo).unwrap_or_default();
        let mut cards = Vec::new();
        for (id, (turns, tokens)) in spent {
            let Some(ticket) = tickets.iter().find(|t| t.id == id) else { continue };
            let settings = own(ticket);
            if tokens < settings.ticket_budget {
                continue;
            }
            let record = journal.ticket_entries(id).unwrap_or_default();
            let Some(limit) = spend::limit(&settings, &record).filter(|limit| tokens >= *limit) else { continue };
            cards.push((ticket.clone(), Card::new(ticket, &record, tokens, turns, Trip::Budget { spent: tokens, limit }, &settings)));
        }
        for (ticket, card) in cards {
            let id = ticket.id;
            self.stop_for_budget(squad, ticket, card);
            tickets.retain(|t| t.id != id);
        }
    }

    /// A ticket not running stops as [`Runtime::budget_stop`] stops one that
    /// runs: a proposal again, its branch kept, the PM woken to settle it.
    fn stop_for_budget(&mut self, squad: &str, mut ticket: Ticket, card: Card) {
        crate::daemon_log!("equipe: {squad}: T-{} held before it started again: {}", ticket.id, card.why());
        ticket.status = TicketStatus::Proposed;
        ticket.assignee = None;
        ticket.note = Some(format!("{}{} — {}", crate::protocol::HANDED_BACK, turn_end::Reason::Budget.said(), card.record_line()));
        let Some(journal) = self.journal(squad) else { return };
        let _ = journal.save_ticket(&ticket);
        let _ = journal.ticket_event(ticket.id, crate::lead_time::EQUIPE, spend::STOP, &card.record_line());
        let shown = self.said("news.budget_stop", &[("id", ticket.id.to_string()), ("title", ticket.title.clone())]);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        if let Some(pm) = self.pm_of(squad) {
            self.show_news(pm, shown);
            let prompt = format!("[equipe: {} Settle it now, then carry on with what else waits.]", card.pm_line());
            let _ = self.internal.send(Internal::Wake { session: pm, prompt });
        }
    }

    // ── the minute's look ──

    /// Each minute: whether every plan is nearly out; every
    /// [`health::EVERY`], the squad's spend rate and its self-watch.
    pub(super) async fn guards_tick(&mut self) {
        let now = now_secs();
        let squads = self.squads_with_members();
        for squad in &squads {
            self.headroom_check(squad, now);
        }
        if self.guards.watched_at.is_some_and(|at| at.elapsed() < Duration::from_secs(health::EVERY)) {
            return;
        }
        self.guards.watched_at = Some(std::time::Instant::now());
        // Repeat counts of tickets closed since are forgotten.
        let open: HashSet<(String, TicketId)> =
            squads.iter().flat_map(|s| self.tickets(s).iter().filter(|t| !t.status.is_closed()).map(|t| (s.clone(), t.id)).collect::<Vec<_>>()).collect();
        self.guards.turns.retain(|key, _| open.contains(key));
        for squad in squads {
            self.rate_check(&squad, now);
            self.self_watch(&squad, now).await;
            // Looked at for the next look: git runs off the loop.
            self.look_at_repos(&squad);
        }
    }

    /// The squad's projects' git facts, read off the loop ([`health::repo_facts`]).
    fn look_at_repos(&mut self, squad: &str) {
        if !self.guards.looking.insert(squad.to_owned()) {
            return;
        }
        let projects = self.projects(squad);
        let watch = self.journals.get(squad).map(Journal::deploy_watch).unwrap_or_default();
        let lanes = self.lanes(squad).clone();
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let live = health::DeployWatch { watch: &watch, lanes: &lanes };
            let facts = projects.iter().map(|p| health::repo_facts(p, &live)).collect();
            let _ = internal.send(Internal::RepoFacts { squad, facts });
        });
    }

    /// A squad's projects' git facts, back from [`Runtime::look_at_repos`].
    pub(super) fn repo_facts_read(&mut self, squad: String, facts: Vec<RepoFacts>) {
        self.guards.looking.remove(&squad);
        self.guards.repos.insert(squad, facts);
    }

    /// What runs in `squad` now: tickets a turn is on, members at work.
    fn live_of(&self, squad: &str) -> Live {
        let mut live = Live::default();
        for s in self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad)) {
            if !s.is_working() && !s.is_preparing() {
                continue;
            }
            live.any = true;
            if let Some(w) = &s.work {
                live.running.insert(w.ticket);
            }
            let name = s.name().or_else(|| s.run_of.and_then(|of| self.sessions.get(&of)).and_then(|m| m.name()));
            if let Some(name) = name {
                live.busy.insert(name.to_owned());
            }
        }
        live
    }

    /// The findings of `squad` now, from what the daemon knows ([`health::detect`]).
    pub(super) fn health_of(&mut self, squad: &str, now: u64) -> Vec<Finding> {
        let tickets = self.tickets(squad).to_vec();
        let live = self.live_of(squad);
        let asks = self.waiting_asks(squad);
        let repos = self.guards.repos.get(squad).cloned().unwrap_or_default();
        let Some(journal) = self.journal(squad) else { return Vec::new() };
        health::detect(&Snapshot::gather(journal, tickets, now, live, &asks, repos))
    }

    /// The squad looks at itself: what it finds that wasn't told goes to the
    /// PM's brief and to the person as news, each once until it changes. An
    /// idle squad wakes its PM: nothing else would read its brief.
    async fn self_watch(&mut self, squad: &str, now: u64) {
        let found = self.health_of(squad, now);
        let Some(journal) = self.journal(squad) else { return };
        let mut told = journal.health_told();
        let before = told.clone();
        let fresh = told.fresh(&found);
        if told != before
            && let Err(err) = journal.set_health_told(&told)
        {
            crate::daemon_log!("equipe: {squad}: what the self-watch told wasn't kept: {err}");
        }
        let words = crate::i18n::core(self.config.lang);
        // What waits on the person is told by its own list (`tell_waiting_on_person`): not twice.
        for finding in fresh.into_iter().filter(|f| !matches!(f, Finding::Waiting { .. })) {
            crate::daemon_log!("equipe: {squad}: self-watch: {}", finding.key());
            let line = format!("[equipe self-watch] {}", finding.pm_line());
            let shown = finding.said(words);
            match (&finding, self.pm_of(squad)) {
                (Finding::Idle { .. }, Some(pm)) => {
                    self.show_news(pm, shown);
                    // On free credit an idle board is the person's to fill: no PM turn about it.
                    if !self.free_hold(squad, "the self-watch's idle note") {
                        self.hand_to_pm(squad, pm, line).await;
                    }
                }
                _ => self.brief_pm(squad, line, Some(shown)),
            }
        }
    }

    /// The last hour's spend against the week's hourly median: one line to
    /// the PM and the person when it is high, at most once an hour.
    fn rate_check(&mut self, squad: &str, now: u64) {
        let Some(journal) = self.journal(squad) else { return };
        let turns = journal.turn_spend_since(now.saturating_sub(spend::RATE_WINDOW)).unwrap_or_default();
        let rate = spend::rate(&turns, now);
        if !self.guards.rate.entry(squad.to_owned()).or_default().step(&rate, now) {
            return;
        }
        let median = rate.median.unwrap_or(1).max(1);
        let times = format!("{:.1}", rate.last_hour as f64 / median as f64);
        let tickets = self.tickets(squad);
        let named: Vec<String> = rate
            .top
            .iter()
            .map(|(id, tokens)| {
                let title = tickets.iter().find(|t| t.id == *id).map(|t| t.title.clone()).unwrap_or_default();
                format!("T-{id} {title} ({})", crate::usage::amount(*tokens))
            })
            .collect();
        let named = if named.is_empty() { "-".to_owned() } else { named.join(", ") };
        crate::daemon_log!("equipe: {squad}: spend rate {} in the last hour, {times}× the hourly median", rate.last_hour);
        let line = format!(
            "[equipe] The squad spent {} tokens in the last hour, {times}× its usual hour (the week's median, {}). Most of it went to: {named}. \
If one of them is going round in circles, stop it or narrow it.",
            crate::usage::amount(rate.last_hour),
            crate::usage::amount(median)
        );
        let shown = self.said("news.spend_rate", &[("tokens", crate::usage::amount(rate.last_hour)), ("times", times), ("tickets", named)]);
        self.brief_pm(squad, line, Some(shown));
    }

    /// Every signed-in engine's week nearly out: told once per level, with
    /// what still has room and the low-priority work to put off.
    fn headroom_check(&mut self, squad: &str, now: u64) {
        // No plan read near its week's end: nothing to look at, and no need to ask which engines are signed in.
        let first = spend::HEADROOM_LEVELS[0];
        let near = self.plans.iter().any(|p| spend::Room { engine: p.engine, week: p.usage.seven_day }.used(now as i64).is_some_and(|u| u >= first));
        if !near {
            self.guards.headroom.remove(squad);
            return;
        }
        let rooms: Vec<spend::Room> = self
            .reach()
            .ready
            .iter()
            .map(|e| spend::Room { engine: *e, week: self.plans.iter().find(|p| p.engine == *e).and_then(|p| p.usage.seven_day) })
            .collect();
        let level = spend::headroom_level(&rooms, now as i64);
        let told = self.guards.headroom.entry(squad.to_owned()).or_default();
        let Some(level) = spend::headroom_step(told, level) else { return };
        let plans: Vec<String> = rooms
            .iter()
            .map(|r| {
                let used = r.used(now as i64).unwrap_or(0);
                let reset = r.week.map(|w| w.resets_at).filter(|at| *at > now as i64).map(|at| format!(", resets in {}", crate::logins::span(at as u64 - now))).unwrap_or_default();
                format!("{} {used}%{reset}", r.engine.label())
            })
            .collect();
        let room_left: Vec<String> = rooms.iter().filter(|r| r.used(now as i64).is_some_and(|u| u < 100)).map(|r| r.engine.label().to_owned()).collect();
        let running = self.live_of(squad).running.len();
        let tickets = self.tickets(squad);
        let low: Vec<String> =
            tickets.iter().filter(|t| t.status == TicketStatus::Todo && t.priority == equipe_protocol::Priority::Low).take(5).map(|t| format!("T-{} {}", t.id, t.title)).collect();
        crate::daemon_log!("equipe: {squad}: every plan is {level}% or more through its week");
        let still = if room_left.is_empty() { "Nothing has room left until a reset".to_owned() } else { format!("Still with room: {}", room_left.join(", ")) };
        let defer = if low.is_empty() { String::new() } else { format!(" Low-priority work to put off: {}.", low.join(", ")) };
        let line = format!(
            "[equipe] Every signed-in plan is at least {level}% through its week: {}. {still}; {running} tickets run now. The squad stops when \
they run out: put low-priority tickets off until the reset, and keep what is left for what matters most.{defer}",
            plans.join("; ")
        );
        let shown = self.said("news.headroom", &[("level", level.to_string()), ("plans", plans.join(", ")), ("count", running.to_string())]);
        self.brief_pm(squad, line, Some(shown));
    }
}
