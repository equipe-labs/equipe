//! The growth loop's runtime side (`crate::growth` has the rules): a run
//! gathers the squad's numbers off the loop and hands them to the PM in one
//! message; the PM's cards go to the person on one decision card; their
//! answers are carried out — a ticket for each accepted card, serving a
//! request of its own — and each minute equipe looks for accepted cards gone
//! live and follow-ups due, measured off the loop.

use equipe_protocol::decide::{Answer, QuestionSet};
use equipe_protocol::growth::{Better, Card, Evidence, Growth, GrowthParams, Measure, Outcome, RunStarted, Status};
use equipe_protocol::metrics::{HogqlParams, QueryParams, Shape, SourceKind};
use equipe_protocol::{Priority, TicketDraft};

use super::*;
use crate::growth::{self as rules, Config, Store};

/// What a run brought back from off the loop.
#[derive(Debug, Default)]
pub(super) struct Gathered {
    pub evidence: Vec<Evidence>,
    pub gaps: Vec<String>,
    pub by: &'static str,
}

/// How long one health check may take.
const HEALTH_WAIT: Duration = Duration::from_secs(10);
/// A weekly run that couldn't start is tried again after this long.
const RETRY: u64 = 60 * 60;

impl Runtime {
    fn growth_dir(&self, squad: &str) -> PathBuf {
        crate::journal::squad_dir(&self.config.home, squad)
    }

    fn keep_growth(&self, squad: &str, store: &Store) {
        if let Err(why) = rules::save(&self.growth_dir(squad), store) {
            crate::daemon_log!("equipe: {squad}: the growth cards weren't kept: {why}");
        }
    }

    /// The squad's projects' declarations as one, and what couldn't be read.
    pub(super) fn growth_config(&mut self, squad: &str) -> (Config, Vec<String>, bool) {
        let mut gaps = Vec::new();
        let mut found = Vec::new();
        for project in self.projects(squad) {
            match rules::config(&project) {
                Ok(Some(c)) => found.push(c),
                Ok(None) => {}
                Err(why) => gaps.push(format!("{}: {why}", board::project_name(&project))),
            }
        }
        let declared = !found.is_empty();
        (Config::merged(found), gaps, declared)
    }

    /// The squad's PostHog connection a run reads: the one its declaration
    /// names, or its only one.
    pub(super) fn growth_connection(&self, squad: &str, config: &Config) -> Result<Option<String>, String> {
        let all: Vec<String> = crate::metrics::connections(&self.config.home).into_iter().filter(|c| c.squad == squad && c.kind == SourceKind::Posthog).map(|c| c.id).collect();
        match (&config.connection, all.as_slice()) {
            (Some(named), _) if all.contains(named) => Ok(Some(named.clone())),
            (Some(named), _) => Err(format!("the connection {named} named in {} isn't one of the squad's PostHog connections", rules::FILE)),
            (None, []) => Ok(None),
            (None, [only]) => Ok(Some(only.clone())),
            (None, _) => Err(format!("the squad has several PostHog connections: name one as `connection` in {}", rules::FILE)),
        }
    }

    /// Start a run: the numbers gathered off the loop, then handed to the PM
    /// ([`Runtime::growth_gathered`]). `by`: `weekly` or `asked`.
    pub(super) fn growth_run(&mut self, squad: &str, by: &'static str) -> RunStarted {
        let now = now_secs();
        let refused = |why: String| RunStarted { started: false, why: Some(why) };
        if !self.journals.contains_key(squad) && self.journal(squad).is_none() {
            return refused(format!("there is no squad {squad}"));
        }
        if self.pm_of(squad).is_none() {
            return refused("the squad has no PM to turn the numbers into cards".into());
        }
        if self.growth_running.contains(squad) {
            return refused("a growth run is under way".into());
        }
        let store = rules::load(&self.growth_dir(squad));
        let (config, mut gaps, _) = self.growth_config(squad);
        if let Err(why) = rules::may_spend(&store, config.weekly_tokens(), rules::TURN_ALLOWANCE, now) {
            return refused(why);
        }
        // Cards gone live meanwhile are looked at first.
        self.growth_look(squad, now);
        let connection = match self.growth_connection(squad, &config) {
            Ok(c) => c,
            Err(why) => {
                gaps.push(why);
                None
            }
        };
        let asks = match &connection {
            Some(c) => rules::asks(&config, c).unwrap_or_else(|why| {
                gaps.push(format!("{}: {why}", rules::FILE));
                Vec::new()
            }),
            None => Vec::new(),
        };
        if connection.is_some() && config.key_events.is_empty() && config.funnels.is_empty() {
            gaps.push(format!("no key events or funnels declared in {}", rules::FILE));
        }
        // What equipe knows without asking anyone: its signals and the project checks it ran.
        let mut local = self.growth_signals(squad, now);
        local.extend(self.growth_checks(squad, now));
        let (home, squad_name, internal, client) = (self.config.home.clone(), squad.to_owned(), self.internal.clone(), self.client.clone());
        let health = config.health.clone();
        self.growth_running.insert(squad.to_owned());
        crate::daemon_log!("equipe: {squad}: a growth run starts ({by}; {} queries)", asks.len());
        tokio::spawn(async move {
            let mut gathered = gather(&home, &squad_name, connection.as_deref(), &asks, &health, &client, now).await;
            gathered.evidence.extend(local);
            gathered.gaps.splice(0..0, gaps);
            gathered.by = by;
            let _ = internal.send(Internal::GrowthGathered { squad: squad_name, gathered: Box::new(gathered) });
        });
        RunStarted { started: true, why: None }
    }

    /// A run's numbers back: kept as what cards may cite, cards still
    /// "later" asked again, and — within the week's budget — handed to the
    /// PM in one message.
    pub(super) async fn growth_gathered(&mut self, squad: &str, gathered: Gathered) {
        self.growth_running.remove(squad);
        let now = now_secs();
        let Some(pm) = self.pm_of(squad) else { return };
        let (config, _, _) = self.growth_config(squad);
        let mut store = rules::load(&self.growth_dir(squad));
        let mut evidence = gathered.evidence;
        evidence.truncate(rules::EVIDENCE);
        for (n, e) in evidence.iter_mut().enumerate() {
            e.id = format!("E{}", n + 1);
        }
        if evidence.is_empty() {
            crate::daemon_log!("equipe: {squad}: a growth run found no numbers ({})", gathered.gaps.join("; "));
            let shown = say!(self, "news.growth_nothing", why = gathered.gaps.first().cloned().unwrap_or_default());
            self.show_news(pm, shown);
            return;
        }
        // "Later" cards: asked again with this run's, or let go once old.
        let mut again = Vec::new();
        for card in store.cards.iter_mut().filter(|c| c.status == Status::Later) {
            if card.proposed_at + rules::LATER_KEPT <= now {
                card.status = Status::Dropped;
            } else {
                again.push(card.id);
            }
        }
        let open: Vec<&Card> = store.cards.iter().filter(|c| c.status.is_open()).collect();
        let today = crate::metrics::day_string(crate::metrics::day_of(&routines::Local, now));
        let message = rules::message(&evidence, &open, &gathered.gaps, config.cards().saturating_sub(open.len()), &today);
        let estimate = rules::estimate(&message);
        if let Err(why) = rules::may_spend(&store, config.weekly_tokens(), estimate, now) {
            crate::daemon_log!("equipe: {squad}: the growth run isn't handed over: {why}");
            self.show_news(pm, say!(self, "news.growth_budget"));
            return;
        }
        let count = evidence.len();
        store.evidence = evidence;
        store.last_run = Some(now);
        store.runs.retain(|r| r.at + 8 * rules::WEEK > now);
        store.runs.push(rules::Run { at: now, tokens: estimate, evidence: count, by: gathered.by.to_owned() });
        if !again.is_empty() {
            let cards: Vec<Card> = store.cards.iter().filter(|c| again.contains(&c.id)).cloned().collect();
            let member = self.sessions.get(&pm).and_then(Session::name).unwrap_or("pm").to_owned();
            self.ask_cards(squad, pm, &member, cards, &mut store);
        }
        self.keep_growth(squad, &store);
        crate::daemon_log!("equipe: {squad}: growth run handed to the PM ({count} numbers, ~{estimate} tokens)");
        self.show_news(pm, say!(self, "news.growth_run", count = count));
        match self.sessions.get(&pm) {
            Some(s) if s.is_working() => self.leave_memo(pm, message),
            _ => self.submit(pm, message).await,
        }
    }

    /// `propose_growth`: the PM's cards checked ([`rules::cards`]) and put to
    /// the person on one decision card.
    pub(super) fn propose_growth(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM proposes growth cards; tell the PM what the numbers show.".into());
        }
        let now = now_secs();
        let mut store = rules::load(&self.growth_dir(squad));
        let (config, _, _) = self.growth_config(squad);
        let projects: Vec<String> = self.projects(squad).iter().map(|p| board::project_name(p)).collect();
        let made = rules::cards(args, &store, &config, &projects, now)?;
        let (first, last, count) = (made[0].id, made[made.len() - 1].id, made.len());
        self.ask_cards(squad, id, member, made, &mut store);
        self.keep_growth(squad, &store);
        crate::daemon_log!("equipe: {squad}: {count} growth cards put to the person");
        Ok(format!(
            "Proposed {count} card{} (G-{first}{}): the person answers each — accept, later or no — when they can; their answers \
come as their next message. Don't make tickets for them: equipe makes the accepted ones' tickets and measures them after they go live.",
            if count == 1 { "" } else { "s" },
            if first == last { String::new() } else { format!(" to G-{last}") }
        ))
    }

    /// `cards` put to the person on one decision set, asked by `session`.
    fn ask_cards(&mut self, squad: &str, session: SessionId, member: &str, mut cards: Vec<Card>, store: &mut Store) {
        let questions = cards.iter().map(|c| rules::question(c, self.config.lang)).collect();
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session, member: member.to_owned(), ticket: None, questions, asked_at: now_secs(), review: false, retro: None };
        for (n, card) in cards.iter_mut().enumerate() {
            card.set = set.id;
            card.question = n;
            card.status = Status::Open;
        }
        for card in cards {
            store.next = store.next.max(card.id);
            match store.card(card.id) {
                Some(kept) => *kept = card,
                None => store.cards.push(card),
            }
        }
        equipe_protocol::decide::add(self.question_sets(squad), set.clone());
        self.save_questions(squad);
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: set });
    }

    /// The person answered (or put away) `set`: when it holds growth cards,
    /// each is carried out — accepted: a ticket serving a request of its own;
    /// later: asked again with the next run; no: remembered. The message for
    /// the asker in place of the plain answers; `None`: not growth's.
    pub(super) async fn growth_settled(&mut self, squad: &str, set: &QuestionSet, answers: Option<&[Answer]>) -> Option<String> {
        let mut store = rules::load(&self.growth_dir(squad));
        let ids: Vec<u32> = store.cards.iter().filter(|c| c.set == set.id && c.status == Status::Open).map(|c| c.id).collect();
        if ids.is_empty() {
            return None;
        }
        let now = now_secs();
        let mut lines = Vec::new();
        let mut made = false;
        for id in ids {
            let Some(card) = store.cards.iter().find(|c| c.id == id).cloned() else { continue };
            let status = rules::answered(answers.and_then(|a| a.get(card.question)));
            let line = match status {
                Status::Accepted => {
                    let (title, body) = rules::ticket_draft(&card);
                    let project = card.project.as_ref().and_then(|p| self.projects(squad).into_iter().find(|path| &board::project_name(path) == p));
                    let draft = TicketDraft { title, body, project, assignee: None, priority: Priority::default(), depends_on: Vec::new(), kind: None };
                    match self.new_ticket(squad, draft) {
                        Ok(ticket) => {
                            made = true;
                            let request = self.journal(squad).and_then(|j| j.requests().of_ticket(ticket));
                            if let Some(kept) = store.card(id) {
                                kept.ticket = Some(ticket);
                                kept.request = request;
                            }
                            let serves = request.map(|r| format!(", serving R-{r}")).unwrap_or_default();
                            format!("G-{id} \"{}\": accepted → T-{ticket}{serves}", card.title)
                        }
                        Err(why) => format!("G-{id} \"{}\": accepted, but its ticket couldn't be made ({why}) — make it yourself", card.title),
                    }
                }
                Status::Declined => {
                    rules::decline(&mut store, &card, now);
                    format!("G-{id} \"{}\": no — not proposed again for {} days", card.title, rules::DECLINED_FOR / rules::DAY)
                }
                _ => format!("G-{id} \"{}\": later — asked again with the next run's cards", card.title),
            };
            if let Some(kept) = store.card(id) {
                kept.status = status;
                kept.answered_at = Some(now);
            }
            lines.push(line);
        }
        self.keep_growth(squad, &store);
        if made {
            self.dispatch(squad).await;
        }
        Some(rules::answers_message(&lines))
    }

    /// Each minute: weekly runs due, cards gone live, follow-ups due.
    pub(super) fn growth_tick(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let last = rules::load(&self.growth_dir(&squad)).last_run;
            let tried = self.growth_tried.get(&squad).is_some_and(|at| at + RETRY > now);
            // While the sprint loop runs, its review reads the numbers: no weekly run beside it.
            if !tried && rules::weekly_due(last, now, &routines::Local) && self.growth_enabled(&squad) && self.sprint_on(&squad).is_err() {
                self.growth_tried.insert(squad.clone(), now);
                let started = self.growth_run(&squad, "weekly");
                if let Some(why) = started.why {
                    crate::daemon_log!("equipe: {squad}: the weekly growth run didn't start: {why}");
                }
            }
            self.growth_look(&squad, now);
        }
    }

    /// Whether the loop runs weekly for `squad`: it has a PostHog connection,
    /// or a project declares what matters.
    pub(super) fn growth_enabled(&mut self, squad: &str) -> bool {
        crate::metrics::connections(&self.config.home).iter().any(|c| c.squad == squad && c.kind == SourceKind::Posthog) || self.growth_config(squad).2
    }

    /// Accepted cards whose ticket went live get their follow-ups; a
    /// cancelled one is dropped; follow-ups due are measured off the loop.
    pub(super) fn growth_look(&mut self, squad: &str, now: u64) {
        let dir = self.growth_dir(squad);
        if !rules::store_path(&dir).exists() {
            return;
        }
        let mut store = rules::load(&dir);
        if !store.cards.iter().any(|c| matches!(c.status, Status::Accepted | Status::Live)) {
            return;
        }
        let deploys = self.journal(squad).map(Journal::deploys).unwrap_or_default();
        let tickets = self.tickets(squad);
        let mut changed = false;
        let mut live = Vec::new();
        for card in store.cards.iter_mut().filter(|c| c.status == Status::Accepted) {
            let Some(ticket) = card.ticket.and_then(|t| tickets.iter().find(|x| x.id == t)) else { continue };
            if ticket.status == TicketStatus::Cancelled {
                card.status = Status::Dropped;
                changed = true;
            } else if let Some(at) = rules::live_at(ticket, &deploys, now) {
                rules::went_live(card, at);
                live.push((card.id, card.title.clone(), ticket.id));
                changed = true;
            }
        }
        if changed {
            self.keep_growth(squad, &store);
        }
        for (id, title, ticket) in live {
            crate::daemon_log!("equipe: {squad}: growth card G-{id} is live (T-{ticket}); measured in 7 and 14 days");
            let line = format!("G-{id} \"{title}\" is live with T-{ticket}: equipe reads its metric again in 7 and 14 days.");
            let shown = say!(self, "news.growth_live", id = id, title = title, ticket = ticket);
            self.brief_pm(squad, line, Some(shown));
        }
        let due: Vec<(Card, u32)> = store.cards.iter().flat_map(|c| rules::due(c, now).into_iter().map(move |d| (c.clone(), d))).collect();
        for (card, after_days) in due {
            if self.growth_measuring.insert((squad.to_owned(), card.id, after_days)) {
                self.growth_measure(squad, card, after_days);
            }
        }
    }

    /// Read a live card's metric again, off the loop: the days before it went
    /// live beside as many after.
    fn growth_measure(&mut self, squad: &str, card: Card, after_days: u32) {
        let Some(live) = card.live_at else { return };
        let ((bf, bt), (af, at)) = rules::windows(live, after_days);
        let base = card.evidence.iter().find(|e| e.metric == card.metric).or(card.evidence.first()).cloned();
        let (home, squad_name, internal) = (self.config.home.clone(), squad.to_owned(), self.internal.clone());
        let id = card.id;
        tokio::spawn(async move {
            let day = crate::metrics::day_string;
            let mut outcome = Outcome {
                verdict: equipe_protocol::growth::Verdict::NoChange,
                before: 0.0,
                after: 0.0,
                change_pct: None,
                before_from: day(bf),
                before_to: day(bt),
                after_from: day(af),
                after_to: day(at),
                query: String::new(),
                at: now_secs(),
                failed: None,
            };
            let measured = match base.as_ref().and_then(|e| e.measure.clone().map(|m| (m, e.better))) {
                None => Err("its numbers can't be read again".to_owned()),
                Some((measure, better)) => {
                    outcome.query = rules::measure_query(&measure, af, at).unwrap_or_else(|| "equipe's kept snapshots".into());
                    match (read(&home, &squad_name, &measure, bf, bt).await, read(&home, &squad_name, &measure, af, at).await) {
                        (Ok(before), Ok(after)) => Ok((before, after, better)),
                        (Err(e), _) | (_, Err(e)) => Err(e),
                    }
                }
            };
            match measured {
                Ok(((before, n0), (after, n1), better)) => {
                    let (verdict, pct) = rules::classify(before, after, n0.min(n1), better);
                    outcome = Outcome { verdict, before, after, change_pct: pct, ..outcome };
                }
                Err(why) => outcome.failed = Some(why),
            }
            let _ = internal.send(Internal::GrowthMeasured { squad: squad_name, card: id, after_days, outcome: Box::new(outcome) });
        });
    }

    /// A follow-up back: kept on its card, told to the person and the PM;
    /// worse brings a card to undo it.
    pub(super) fn growth_measured(&mut self, squad: &str, id: u32, after_days: u32, outcome: Outcome) {
        self.growth_measuring.remove(&(squad.to_owned(), id, after_days));
        let mut store = rules::load(&self.growth_dir(squad));
        let Some(card) = store.card(id) else { return };
        rules::measured(card, after_days, outcome.clone());
        let card = card.clone();
        let line = rules::outcome_line(&card, after_days, &outcome);
        crate::daemon_log!("equipe: {squad}: {line}");
        let pct = outcome.change_pct.map(|p| format!("{}{}%", if p >= 0.0 { "+" } else { "" }, rules::round1(p))).unwrap_or_else(|| "–".into());
        let (before, after) = (rules::round1(outcome.before), rules::round1(outcome.after));
        let shown = match (&outcome.failed, outcome.verdict) {
            (Some(_), _) => say!(self, "news.growth_unmeasured", id = id, title = card.title, days = after_days),
            (None, equipe_protocol::growth::Verdict::Worked) => say!(self, "news.growth_worked", id = id, title = card.title, days = after_days, before = before, after = after, pct = pct),
            (None, equipe_protocol::growth::Verdict::NoChange) => say!(self, "news.growth_no_change", id = id, title = card.title, days = after_days, before = before, after = after, pct = pct),
            (None, equipe_protocol::growth::Verdict::Worse) => say!(self, "news.growth_worse", id = id, title = card.title, days = after_days, before = before, after = after, pct = pct),
        };
        let worse = outcome.failed.is_none() && outcome.verdict == equipe_protocol::growth::Verdict::Worse;
        let undone = store.cards.iter().any(|c| c.reverts == Some(id));
        let mut brief = line;
        if worse && card.reverts.is_none() && !undone
            && let Some(pm) = self.pm_of(squad)
        {
            let undo = rules::revert_card(&card, &outcome, store.next + 1, now_secs());
            brief.push_str(&format!(" equipe put G-{} to the person: undo it, or try another way.", undo.id));
            let member = self.sessions.get(&pm).and_then(Session::name).unwrap_or("pm").to_owned();
            self.ask_cards(squad, pm, &member, vec![undo], &mut store);
        }
        self.keep_growth(squad, &store);
        self.brief_pm(squad, brief, Some(shown));
    }

    /// A screen's `growth.*` request: its answer, or a JSON-RPC error.
    pub(super) fn growth_request(&mut self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        let p: GrowthParams = serde_json::from_value(params).map_err(|e| (-32602, format!("{method}: {e}")))?;
        if !crate::squads::valid_name(&p.squad) || self.journal(&p.squad).is_none() {
            return Err((-32602, format!("there is no squad {}", p.squad)));
        }
        let answer = match method {
            equipe_protocol::growth::LIST => serde_json::to_value(self.growth_view(&p.squad)),
            equipe_protocol::growth::RUN => serde_json::to_value(self.growth_run(&p.squad, "asked")),
            _ => return Err((-32601, "unknown method".into())),
        };
        answer.map_err(|e| (-32000, e.to_string()))
    }

    /// The squad's loop as screens show it.
    pub(super) fn growth_view(&mut self, squad: &str) -> Growth {
        let store = rules::load(&self.growth_dir(squad));
        let (config, _, _) = self.growth_config(squad);
        let mut cards = store.cards.clone();
        cards.sort_by_key(|c| std::cmp::Reverse(c.id));
        Growth { cards, last_run: store.last_run, spent: store.spent(now_secs()), cap: config.weekly_tokens() }
    }

    /// The squad's signals that failed in the last week: the share of their
    /// readings that passed.
    pub(super) fn growth_signals(&mut self, squad: &str, now: u64) -> Vec<Evidence> {
        let states = self.journal(squad).map(Journal::signal_states).unwrap_or_default();
        let signals = self.journal(squad).map(Journal::signals).unwrap_or_default();
        let (from, to) = (crate::metrics::day_of(&routines::Local, now.saturating_sub(rules::WEEK)), crate::metrics::day_of(&routines::Local, now));
        signals
            .iter()
            .filter_map(|s| {
                let readings: Vec<_> = states.get(&s.key())?.readings.iter().filter(|r| r.at + rules::WEEK > now).collect();
                let ok = readings.iter().filter(|r| r.ok).count();
                (!readings.is_empty() && ok < readings.len()).then(|| Evidence {
                    id: String::new(),
                    metric: format!("signal:{}/{}", s.project, s.name),
                    label: format!("signal {} of {} passed ({ok} of {} readings)", s.name, s.project, readings.len()),
                    value: rules::round1(ok as f64 / readings.len() as f64 * 100.0),
                    previous: None,
                    unit: "%".into(),
                    from: crate::metrics::day_string(from),
                    to: crate::metrics::day_string(to),
                    query: format!("signal `{}`", s.name),
                    source: "equipe signals".into(),
                    better: Better::Up,
                    measure: None,
                })
            })
            .collect()
    }

    /// The project checks equipe ran on accepted work in the last week: the
    /// share that passed, when any ran.
    pub(super) fn growth_checks(&mut self, squad: &str, now: u64) -> Vec<Evidence> {
        let since = now.saturating_sub(rules::WEEK);
        let events = self.journal(squad).and_then(|j| j.ticket_events_since(since).ok()).unwrap_or_default();
        let passed = events.iter().filter(|(_, e)| e.kind == "checks passed").count();
        let failed = events.iter().filter(|(_, e)| e.kind == "checks failed").count();
        if passed + failed == 0 {
            return Vec::new();
        }
        let (from, to) = (crate::metrics::day_of(&routines::Local, since), crate::metrics::day_of(&routines::Local, now));
        vec![Evidence {
            id: String::new(),
            metric: "checks:passed".into(),
            label: format!("project checks passed on accepted work ({passed} of {})", passed + failed),
            value: rules::round1(passed as f64 / (passed + failed) as f64 * 100.0),
            previous: None,
            unit: "%".into(),
            from: crate::metrics::day_string(from),
            to: crate::metrics::day_string(to),
            query: "the squad's journal: checks passed / failed".into(),
            source: "equipe checks".into(),
            better: Better::Up,
            measure: None,
        }]
    }
}

/// A run's numbers, off the loop: each ask this period and the one before
/// through the squad's PostHog connection, equipe's kept daily metrics, and
/// the project's health checks.
pub(super) async fn gather(home: &Path, squad: &str, connection: Option<&str>, asks: &[rules::Ask], health: &[rules::Health], client: &reqwest::Client, now: u64) -> Gathered {
    let mut out = Gathered::default();
    let today = crate::metrics::day_of(&routines::Local, now);
    let (period, before) = rules::periods(today);
    for ask in asks {
        let source = format!("posthog {}", connection.unwrap_or_default());
        match (read(home, squad, &ask.measure, period.0, period.1).await, read(home, squad, &ask.measure, before.0, before.1).await) {
            (Ok(now), before) => out.evidence.push(rules::evidence(0, ask, now, before.ok(), period, &source)),
            (Err(why), _) => out.gaps.push(format!("{}: {why}", ask.metric)),
        }
    }
    // equipe's own snapshots of the squad's connected numbers.
    let q = QueryParams { squad: squad.to_owned(), days: Some(rules::PERIOD as u32), to: Some(crate::metrics::day_string(period.1)), ..QueryParams::default() };
    if let Ok(r) = crate::metrics::query(home, &q, today) {
        for v in r.values.iter().filter(|v| v.shape == Shape::Count) {
            let Some(value) = v.value else { continue };
            out.evidence.push(Evidence {
                id: String::new(),
                metric: format!("{}@{}", v.metric, v.connection),
                label: format!("{} ({} {})", v.label, crate::metrics::kind_label(v.kind), v.connection),
                value,
                previous: v.previous,
                unit: v.unit.clone(),
                from: r.from.clone(),
                to: r.to.clone(),
                query: format!("equipe's daily snapshots of {} · {}", v.connection, v.metric),
                source: format!("metrics {}", v.connection),
                better: Better::Up,
                measure: Some(Measure::Snapshot { connection: v.connection.clone(), metric: v.metric.clone() }),
            });
        }
    }
    for h in health {
        match (&h.url, &h.hogql, connection) {
            (Some(url), _, _) => out.evidence.push(health_url(client, &h.name, url, today).await),
            (None, Some(q), Some(c)) => match hogql_number(home, squad, c, q).await {
                Ok(value) => out.evidence.push(Evidence {
                    id: String::new(),
                    metric: format!("health:{}", h.name),
                    label: format!("health {}", h.name),
                    value,
                    previous: None,
                    unit: String::new(),
                    from: crate::metrics::day_string(today),
                    to: crate::metrics::day_string(today),
                    query: q.clone(),
                    source: format!("posthog {c}"),
                    better: h.better,
                    measure: None,
                }),
                Err(why) => out.gaps.push(format!("health {}: {why}", h.name)),
            },
            _ => out.gaps.push(format!("health {}: needs a url, or a hogql query and a PostHog connection", h.name)),
        }
    }
    out
}

/// A measure over days `from..=to`: its value and how many it counted.
async fn read(home: &Path, squad: &str, measure: &Measure, from: i64, to: i64) -> Result<(f64, f64), String> {
    match measure {
        Measure::Snapshot { connection, metric } => {
            let q = QueryParams {
                squad: squad.to_owned(),
                days: Some((to - from + 1).clamp(1, 366) as u32),
                to: Some(crate::metrics::day_string(to)),
                connections: vec![connection.clone()],
                metrics: vec![metric.clone()],
                ..QueryParams::default()
            };
            let r = crate::metrics::query(home, &q, to + 1).map_err(|e| e.message)?;
            let v = r.values.first().and_then(|v| v.value).ok_or("no kept numbers for those days")?;
            Ok((v, v))
        }
        Measure::Event { connection, .. } | Measure::Funnel { connection, .. } => {
            let query = rules::measure_query(measure, from, to).ok_or("no query")?;
            let p = HogqlParams { squad: squad.to_owned(), connection: Some(connection.clone()), project: None, query, limit: Some(10) };
            let r = crate::metrics::shared(home).hogql(&p, now_secs()).await.map_err(|e| e.message)?;
            rules::read_value(measure, &r.rows).ok_or_else(|| "PostHog's rows weren't what was asked".into())
        }
    }
}

/// A health query's one number.
async fn hogql_number(home: &Path, squad: &str, connection: &str, query: &str) -> Result<f64, String> {
    let p = HogqlParams { squad: squad.to_owned(), connection: Some(connection.to_owned()), project: None, query: query.to_owned(), limit: Some(1) };
    let r = crate::metrics::shared(home).hogql(&p, now_secs()).await.map_err(|e| e.message)?;
    let cell = r.rows.first().and_then(|row| row.first()).ok_or("no rows")?;
    cell.as_f64().or_else(|| cell.as_str()?.parse().ok()).ok_or_else(|| "its first cell isn't a number".into())
}

/// An https health check: up (1) or not (0), with its status and time.
async fn health_url(client: &reqwest::Client, name: &str, url: &str, today: i64) -> Evidence {
    let started = std::time::Instant::now();
    let (value, said) = if !url.starts_with("https://") {
        (0.0, "not an https address".to_owned())
    } else {
        match client.get(url).timeout(HEALTH_WAIT).send().await {
            Ok(r) => (if r.status().is_success() { 1.0 } else { 0.0 }, format!("HTTP {} in {} ms", r.status().as_u16(), started.elapsed().as_millis())),
            Err(e) if e.is_timeout() => (0.0, format!("no answer in {}s", HEALTH_WAIT.as_secs())),
            Err(_) => (0.0, "unreachable".to_owned()),
        }
    };
    Evidence {
        id: String::new(),
        metric: format!("health:{name}"),
        label: format!("health {name}: {said}"),
        value,
        previous: None,
        unit: "up".into(),
        from: crate::metrics::day_string(today),
        to: crate::metrics::day_string(today),
        query: format!("GET {url}"),
        source: "health".into(),
        better: Better::Up,
        measure: None,
    }
}
