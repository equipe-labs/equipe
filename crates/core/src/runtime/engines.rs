//! Both plans, watched; members moved between engines at their ticket
//! boundaries. Each engine's usage comes in as [`Event::PlanUsage`] and is
//! kept with when it was read; the squad's rules (`crate::switching`) say what
//! to tell, hold and move, and this makes it so:
//!
//! - in `suggest`, a card the person answers ([`Command::ApplyEngineProposal`]);
//!   in `auto`, the moves are made and the card tells of them, with an undo;
//! - a move waits for its member's boundary: idle between tickets, or before
//!   it picks up the next one ([`Runtime::dispatch`] asks here first) — never
//!   mid-turn; an exhausted engine's work is handed over between turns;
//! - moving is what `/close` and `/add` do ([`Command::Close`], then
//!   [`Command::Start`] on the other engine, with the same name), after a WIP
//!   commit on the ticket's branch, and the new session's first message is a
//!   brief equipe writes: the ticket, what is done, what is next, what is open.
//!
//! Every decision goes to the squad's journal and to the PM's conversation as
//! news. Nothing here spends credits or overage: an engine out of its plan is
//! waited for or left, never paid for.

use equipe_protocol::engines::{Move, Proposal, Switching, Why, Window};
use equipe_protocol::{EnginePlan, PlanUsage};
use serde::{Deserialize, Serialize};

use super::*;
use crate::switching::{self as rules, Hold, Level, Mate, Memory, Reach, Reading};

/// How long a look at whether Claude Code is signed in holds.
const READY_FOR: Duration = Duration::from_secs(10 * 60);
/// The decisions the journal keeps, newest last.
const LOG_KEPT: usize = 200;
/// How much of the conversation a brief carries, in characters (~2–4k tokens in all).
const BRIEF_TALK: usize = 6_000;
const BRIEF_BODY: usize = 3_000;

/// What engine switching keeps in memory, for every squad.
#[derive(Default)]
pub(super) struct Engines {
    /// Each engine's plan, as last read.
    plans: HashMap<Engine, EnginePlan>,
    squads: HashMap<String, SquadEngines>,
    /// Sessions about to start for a member moving engines: the name it keeps.
    names: HashMap<SessionId, String>,
    /// The brief each such session's first turn opens with.
    briefs: HashMap<SessionId, String>,
    /// Moves under way: a boundary reached meanwhile doesn't start more.
    applying: bool,
    /// Whether Claude Code was signed in, and when that was looked at.
    claude_ready: Option<(std::time::Instant, bool)>,
    /// A look at Claude's sign-in is under way, off the loop.
    claude_looking: bool,
}

#[derive(Default)]
struct SquadEngines {
    memory: Memory,
    /// Cards on screen, oldest first.
    proposals: Vec<Proposal>,
    /// Moves waiting for their member's boundary, in order.
    due: Vec<Due>,
    /// Engines that take no new tickets for now.
    holds: Vec<Hold>,
}

struct Due {
    proposal: u64,
    why: Why,
    mv: Move,
}

/// What the squad's journal keeps about engines.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Record {
    /// Each member's own engine, and its model there: where it goes back to.
    #[serde(default)]
    homes: HashMap<String, Home>,
    /// When each member moved, newest last (unix seconds).
    #[serde(default)]
    switched: HashMap<String, Vec<i64>>,
    /// The setup for both engines was proposed once.
    #[serde(default)]
    setup_shown: bool,
    /// Every decision, newest last: when, and what.
    #[serde(default)]
    log: Vec<(i64, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Home {
    engine: Engine,
    #[serde(default)]
    model: Option<String>,
}

fn unix_now() -> i64 {
    now_secs() as i64
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

/// Whether Codex members can take what Claude members do — kept in step with
/// what [`Runtime::start`] lets a Codex session do. On the person's own Codex
/// CLI (`codex app-server`) they edit and reach the board as Claude's do; a
/// machine without it has no Codex usage read, so nobody moves there anyway.
fn codex_takes() -> (bool, bool) {
    (true, true)
}

impl Engines {
    /// The name a session starting now keeps, when it is a member moving engines.
    pub(super) fn name_for(&mut self, id: SessionId) -> Option<String> {
        self.names.remove(&id)
    }

    /// The brief a moved member's first turn opens with.
    pub(super) fn brief_for(&mut self, id: SessionId) -> Option<String> {
        self.briefs.remove(&id)
    }

    /// The cards a squad's screens show.
    pub(super) fn proposals(&self, squad: &str) -> Vec<Proposal> {
        self.squads.get(squad).map(|s| s.proposals.clone()).unwrap_or_default()
    }

    /// Whether members on `engine` take no new ticket now.
    pub(super) fn held(&self, squad: &str, engine: Engine) -> bool {
        let now = unix_now();
        self.squads.get(squad).is_some_and(|s| s.holds.iter().any(|h| h.engine == engine && h.until > now))
    }

    /// Whether a move waits for `name`.
    fn waits_for(&self, squad: &str, name: &str) -> bool {
        self.squads.get(squad).is_some_and(|s| s.due.iter().any(|d| d.mv.member == name))
    }
}

impl Runtime {
    fn engines_of(&mut self, squad: &str) -> &mut SquadEngines {
        self.engines.squads.entry(squad.to_owned()).or_default()
    }

    fn engines_record(&mut self, squad: &str) -> Record {
        self.journal(squad).and_then(Journal::engines_record).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    fn save_engines_record(&mut self, squad: &str, record: &Record) {
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_engines_record(&serde_json::to_string(record).unwrap_or_default());
        }
    }

    /// A decision, kept in the squad's journal and the daemon's log.
    fn log_engines(&mut self, squad: &str, what: String) {
        crate::daemon_log!("equipe: engines ({squad}): {what}");
        let mut record = self.engines_record(squad);
        record.log.push((unix_now(), what));
        if record.log.len() > LOG_KEPT {
            record.log.drain(..record.log.len() - LOG_KEPT);
        }
        self.save_engines_record(squad, &record);
    }

    fn switching_mode(&mut self, squad: &str) -> Switching {
        self.journal(squad).map(Journal::engine_switching).unwrap_or_default()
    }

    /// The squads with members.
    pub(super) fn squads_with_members(&self) -> Vec<String> {
        let mut squads: Vec<String> = self.sessions.values().filter(|s| s.member.is_some()).filter_map(|s| s.squad.clone()).collect();
        squads.sort();
        squads.dedup();
        squads
    }

    // ── readings ──

    /// An engine's plan just read: kept with the time, and each squad looks again.
    pub(super) fn on_plan(&mut self, engine: Engine, usage: PlanUsage) {
        let at = unix_now();
        let plan_type = self.engines.plans.get(&engine).and_then(|p| p.plan_type.clone());
        let plan = EnginePlan { engine, usage, plan_type, at };
        let reading = Reading::of(&plan);
        self.engines.plans.insert(engine, plan);
        for squad in self.squads_with_members() {
            self.engines_of(&squad).memory.observe(&reading);
            self.consider_engines(&squad);
        }
    }

    /// What each engine can take on: signed in, and what Codex members can do.
    fn reach(&self) -> Reach {
        let mut ready = Vec::new();
        let claude_on = self.sessions.values().any(|s| s.engine == Engine::Claude) || self.engines.plans.contains_key(&Engine::Claude);
        if claude_on || self.engines.claude_ready.is_some_and(|(_, ok)| ok) {
            ready.push(Engine::Claude);
        }
        if matches!(self.config.store.load("openai"), Ok(Some(_))) {
            ready.push(Engine::Codex);
        }
        let (codex_edits, codex_board) = codex_takes();
        Reach { ready, codex_edits, codex_board }
    }

    /// The squad's members as the rules see them.
    fn mates(&mut self, squad: &str) -> Vec<Mate> {
        let record = self.engines_record(squad);
        let mut team: Vec<(u64, Mate)> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad) && s.lane.is_none())
            .filter_map(|s| {
                let m = s.member.as_ref()?;
                let home = record.homes.get(&m.name);
                Some((
                    s.joined,
                    Mate {
                        name: m.name.clone(),
                        role: m.role.clone(),
                        engine: s.engine,
                        home: home.map_or(s.engine, |h| h.engine),
                        home_model: home.map_or_else(|| s.model.clone(), |h| h.model.clone()),
                        model: s.model.clone(),
                        effort: s.effort.clone(),
                        edits: s.edits(),
                        pinned: false,
                        switched: record.switched.get(&m.name).cloned().unwrap_or_default(),
                        waiting: self.engines.waits_for(squad, &m.name),
                    },
                ))
            })
            .collect();
        team.sort_by_key(|(joined, _)| *joined);
        team.into_iter().map(|(_, m)| m).collect()
    }

    /// Look at `squad` again: tell, hold, propose or move, as its rules say.
    pub(super) fn consider_engines(&mut self, squad: &str) {
        let mode = self.switching_mode(squad);
        let mates = self.mates(squad);
        if mates.is_empty() {
            return;
        }
        let reach = self.reach();
        self.offer_setup(squad, &mates, &reach, false);
        let readings: Vec<Reading> = self.engines.plans.values().map(Reading::of).collect();
        let now = unix_now();
        let decision = {
            let memory = &mut self.engines_of(squad).memory;
            rules::decide(&rules::Squad { readings: &readings, mates: &mates, reach: &reach, mode, now }, memory)
        };
        self.engines_of(squad).holds = decision.hold.clone();
        for engine in &decision.refresh {
            crate::daemon_log!("equipe: engines ({squad}): {} usage is stale or unknown; nothing moves toward it until it is read again", engine_name(*engine));
        }
        for notice in decision.notices {
            self.tell_level(squad, notice);
        }
        if let Some(batch) = decision.batch {
            self.propose_batch(squad, mode, batch);
        }
    }

    /// The person is told, once, where an engine's plan stands.
    fn tell_level(&mut self, squad: &str, n: rules::Notice) {
        let engine = engine_name(n.engine);
        let window = self.window_words(n.window);
        let span = self.span(n.resets_at - unix_now());
        let text = match n.level {
            Level::Exhausted if n.wait => say!(self, "engines.news_exhausted_wait", engine = engine, span = span),
            Level::Exhausted => say!(self, "engines.news_exhausted", engine = engine, span = span),
            _ if n.wait => say!(self, "engines.news_wait_reset", engine = engine, window = window, used = n.used, span = span),
            Level::Watch => match n.runs_out_at {
                Some(at) => say!(self, "engines.news_watch_burn", engine = engine, window = window, used = n.used, span = span, out = self.span(at - unix_now())),
                None => say!(self, "engines.news_watch", engine = engine, window = window, used = n.used, span = span),
            },
            // A shift or a drain is told with what moves (or why nothing can).
            _ => return,
        };
        self.log_engines(squad, format!("{engine} {:?}: {}% of {window}, resets in {span}", n.level, n.used));
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
    }

    fn window_words(&self, window: Option<Window>) -> String {
        match window {
            Some(Window::FiveHour) => say!(self, "engines.window_five_hour"),
            Some(Window::Weekly) => say!(self, "engines.window_weekly"),
            None => say!(self, "engines.window_plan"),
        }
    }

    /// "35분", "2시간", "3일": how long until then.
    fn span(&self, secs: i64) -> String {
        let secs = secs.max(60) as u64;
        match secs {
            ..3_600 => say!(self, "engines.span_minutes", count = secs / 60),
            3_600..172_800 => say!(self, "engines.span_hours", count = (secs + 1_800) / 3_600),
            _ => say!(self, "engines.span_days", count = (secs + 43_200) / 86_400),
        }
    }

    /// Members' names as one phrase: "backend·frontend".
    fn names(moves: &[Move]) -> String {
        moves.iter().map(|m| m.member.as_str()).collect::<Vec<_>>().join("·")
    }

    /// Moves the rules decided on: a card to answer, or — in auto — made at
    /// each member's boundary, and told.
    fn propose_batch(&mut self, squad: &str, mode: Switching, batch: rules::Batch) {
        let open = self.engines_of(squad).proposals.iter().find(|p| !p.applied && p.why != Why::Setup).map(|p| (p.id, p.why));
        match open {
            // Asked already: not again.
            Some((_, why)) if why == batch.why => return,
            // Worse now: the new card takes the old one's place.
            Some((id, _)) => self.close_proposal(squad, id, false),
            None => {}
        }
        let auto = mode == Switching::Auto;
        let proposal = Proposal {
            id: equipe_protocol::new_session_id(),
            why: batch.why,
            engine: Some(batch.engine),
            window: batch.window,
            used_percent: batch.used,
            resets_at: batch.resets_at,
            moves: batch.moves,
            applied: auto,
            proposed_at: now_secs(),
        };
        let text = self.proposal_words(&proposal);
        self.log_engines(squad, format!("{} {:?} ({}): {}", if auto { "made" } else { "proposed" }, proposal.why, engine_name(batch.engine), Self::names(&proposal.moves)));
        if auto {
            self.schedule(squad, &proposal);
        }
        self.engines_of(squad).proposals.push(proposal.clone());
        self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
    }

    /// What a card says, as one line for the news.
    fn proposal_words(&self, p: &Proposal) -> String {
        let names = Self::names(&p.moves);
        let engine = p.engine.map(engine_name).unwrap_or_default();
        let to = p.to().map(engine_name).unwrap_or(engine);
        let window = self.window_words(p.window);
        let span = self.span(p.resets_at - unix_now());
        let used = p.used_percent;
        match (p.why, p.applied) {
            (Why::Setup, _) => say!(self, "engines.news_setup", names = names),
            (Why::StepDown, false) => say!(self, "engines.news_step_down_ask", names = names),
            (Why::StepDown, true) => say!(self, "engines.news_step_down", names = names),
            (Why::Return, _) => say!(self, "engines.news_return", engine = engine, window = window, used = used, names = names),
            (Why::Exhausted, false) => say!(self, "engines.news_hand_over_ask", engine = engine, span = span, names = names, to = to),
            (Why::Exhausted, true) => say!(self, "engines.news_hand_over", engine = engine, span = span, names = names, to = to),
            (_, false) => say!(self, "engines.news_shift_ask", engine = engine, window = window, used = used, span = span, names = names, to = to),
            (_, true) => say!(self, "engines.news_shift", engine = engine, window = window, used = used, names = names, to = to),
        }
    }

    /// A card's moves wait for their members' boundaries.
    fn schedule(&mut self, squad: &str, p: &Proposal) {
        let due = p.moves.iter().filter(|m| !m.done).map(|mv| Due { proposal: p.id, why: p.why, mv: mv.clone() });
        let mut due: Vec<Due> = due.collect();
        let mine = self.engines_of(squad);
        // A newer move for a member replaces one waiting.
        mine.due.retain(|d| due.iter().all(|n| n.mv.member != d.mv.member));
        mine.due.append(&mut due);
        mine.memory.last_batch = Some(unix_now());
    }

    fn close_proposal(&mut self, squad: &str, id: u64, applied: bool) {
        let mine = self.engines_of(squad);
        let before = mine.proposals.len();
        mine.proposals.retain(|p| p.id != id);
        if mine.proposals.len() != before {
            self.publish(Event::EngineProposalClosed { squad: squad.to_owned(), id, applied });
        }
    }

    // ── the setup for both engines ──

    /// The recommended setup, proposed once — or again when the person asks (`again`).
    fn offer_setup(&mut self, squad: &str, mates: &[Mate], reach: &Reach, again: bool) {
        let mut record = self.engines_record(squad);
        if !again && (record.setup_shown || !rules::wants_setup(mates, reach)) {
            return;
        }
        if self.engines_of(squad).proposals.iter().any(|p| p.why == Why::Setup) {
            return;
        }
        let moves = rules::setup(mates, reach);
        if moves.is_empty() {
            let text = say!(self, "engines.setup_already");
            return self.squad_error(squad, text);
        }
        record.setup_shown = true;
        self.save_engines_record(squad, &record);
        let proposal = Proposal { id: equipe_protocol::new_session_id(), why: Why::Setup, engine: None, window: None, used_percent: 0, resets_at: 0, moves, applied: false, proposed_at: now_secs() };
        self.log_engines(squad, format!("proposed the setup for both engines: {}", Self::names(&proposal.moves)));
        self.engines_of(squad).proposals.push(proposal.clone());
        self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
    }

    // ── the person's answers ──

    pub(super) async fn set_engine_switching(&mut self, squad: &str, mode: Switching) {
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_engine_switching(mode);
        }
        self.publish(Event::EngineSwitchingChanged { squad: squad.to_owned(), mode });
        self.log_engines(squad, format!("the person set switching to {}", mode.code()));
        // Off: nothing is proposed any more.
        if mode == Switching::Off {
            let open: Vec<u64> = self.engines_of(squad).proposals.iter().filter(|p| !p.applied && p.why != Why::Setup).map(|p| p.id).collect();
            for id in open {
                self.close_proposal(squad, id, false);
            }
            self.engines_of(squad).holds.clear();
        }
        self.consider_engines(squad);
        self.apply_engine_moves(squad).await;
    }

    /// The person's yes: each move at its member's boundary — now, for those there.
    pub(super) async fn apply_engine_proposal(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.engines_of(squad).proposals.iter().find(|p| p.id == id && !p.applied).cloned() else { return };
        self.schedule(squad, &proposal);
        self.close_proposal(squad, id, true);
        let names = Self::names(&proposal.moves);
        self.log_engines(squad, format!("the person applied {:?}: {names}", proposal.why));
        let text = say!(self, "engines.news_applied", names = names);
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
        self.apply_engine_moves(squad).await;
    }

    /// The person's "leave it": not asked again until the window starts over.
    /// For moves already made, the card goes; the moves stay.
    pub(super) fn dismiss_engine_proposal(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.engines_of(squad).proposals.iter().find(|p| p.id == id).cloned() else { return };
        if !proposal.applied {
            if let Some(engine) = proposal.engine {
                self.engines_of(squad).memory.decline(engine, proposal.why, proposal.resets_at);
            }
            self.log_engines(squad, format!("the person left {:?} as it is", proposal.why));
            if proposal.why != Why::Setup {
                let text = say!(self, "engines.news_declined");
                self.brief_pm(squad, format!("equipe: {text}"), Some(text));
            }
        }
        self.close_proposal(squad, id, proposal.applied);
    }

    /// The person's undo of moves made on their own: what is done goes back
    /// at each member's boundary, what waits doesn't happen.
    pub(super) async fn undo_engine_proposal(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.engines_of(squad).proposals.iter().find(|p| p.id == id).cloned() else { return };
        let mine = self.engines_of(squad);
        mine.due.retain(|d| d.proposal != id);
        if let Some(engine) = proposal.engine {
            mine.memory.decline(engine, proposal.why, proposal.resets_at);
        }
        let back: Vec<Move> = proposal
            .moves
            .iter()
            .filter(|m| m.done)
            .map(|m| Move { from: m.to, to: m.from, model: m.was_model.clone(), effort: None, was_model: m.model.clone(), done: false, ..m.clone() })
            .collect();
        let undo = Proposal { id, moves: back, applied: true, ..proposal.clone() };
        self.schedule(squad, &undo);
        if proposal.why == Why::Setup {
            let mut record = self.engines_record(squad);
            for m in proposal.moves.iter().filter(|m| m.done) {
                record.homes.insert(m.member.clone(), Home { engine: m.from, model: m.was_model.clone() });
            }
            self.save_engines_record(squad, &record);
        }
        self.close_proposal(squad, id, false);
        let names = Self::names(&proposal.moves);
        self.log_engines(squad, format!("the person undid {:?}: {names}", proposal.why));
        let text = say!(self, "engines.news_undone", names = names);
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
        self.apply_engine_moves(squad).await;
    }

    pub(super) fn show_engine_setup(&mut self, squad: &str) {
        let mates = self.mates(squad);
        let reach = self.reach();
        self.offer_setup(squad, &mates, &reach, true);
    }

    // ── at the boundary ──

    /// Each minute: whether Claude Code is signed in (now and then), and every squad looked at again.
    /// Claude's sign-in, as just looked at.
    pub(super) fn claude_ready(&mut self, ok: bool) {
        self.engines.claude_looking = false;
        self.engines.claude_ready = Some((std::time::Instant::now(), ok));
    }

    pub(super) async fn engines_tick(&mut self) {
        // Whether Claude is signed in, asked of `claude auth status` off the
        // loop: it can take seconds (or hang), and the loop answers screens.
        if !self.engines.claude_looking && self.engines.claude_ready.is_none_or(|(at, _)| at.elapsed() >= READY_FOR) {
            self.engines.claude_looking = true;
            let (binary, internal) = (self.config.claude_binary.clone(), self.internal.clone());
            tokio::spawn(async move {
                let ok = matches!(tokio::time::timeout(SERVER_WAIT, claude_code::auth_status(&binary)).await, Ok(Ok(Some(status))) if status.logged_in);
                let _ = internal.send(super::Internal::ClaudeReady(ok));
            });
        }
        for squad in self.squads_with_members() {
            self.consider_engines(&squad);
            self.apply_engine_moves(&squad).await;
        }
    }

    /// Whether member `id` is at a boundary for `due`: not mid-turn, nothing
    /// waiting to be delivered to it or asked of the person; for a move to the
    /// other engine, between tickets too — unless its engine is out and the
    /// work is handed over.
    fn at_boundary(&mut self, squad: &str, id: SessionId, due: &Due) -> bool {
        let Some(s) = self.sessions.get(&id) else { return false };
        if s.is_working() || !s.queued.is_empty() || self.pending.values().any(|p| p.approval.session == id) {
            return false;
        }
        let on_ticket = s.work.is_some();
        let name = due.mv.member.clone();
        if self.question_sets(squad).iter().any(|q| q.member == name) {
            return false;
        }
        due.mv.in_place() || !on_ticket || due.why == Why::Exhausted
    }

    /// Make the moves whose members are at their boundary. Asked before the
    /// board hands out work, so a member moving picks its next ticket up on
    /// the new engine.
    pub(super) async fn apply_engine_moves(&mut self, squad: &str) {
        if self.engines.applying || self.engines.squads.get(squad).is_none_or(|s| s.due.is_empty()) {
            return;
        }
        self.engines.applying = true;
        let mut at = 0;
        while let Some(due) = self.engines.squads.get(squad).and_then(|s| s.due.get(at)) {
            let (proposal, why, mv) = (due.proposal, due.why, due.mv.clone());
            let Some(id) = self.member_id(squad, &mv.member) else {
                // Gone: nothing to move.
                self.engines_of(squad).due.remove(at);
                continue;
            };
            let gone = self.sessions.get(&id).is_some_and(|s| s.engine == mv.to && mv.model.is_none() && mv.effort.is_none());
            let due = Due { proposal, why, mv: mv.clone() };
            if !gone && !self.at_boundary(squad, id, &due) {
                at += 1;
                continue;
            }
            self.engines_of(squad).due.remove(at);
            let moved = gone || self.switch_member(squad, id, &due).await;
            if moved {
                self.move_done(squad, proposal, &mv);
            }
        }
        self.engines.applying = false;
    }

    /// A move made: its card says so, and the member's record.
    fn move_done(&mut self, squad: &str, proposal: u64, mv: &Move) {
        let mut record = self.engines_record(squad);
        if !mv.in_place() {
            record.homes.entry(mv.member.clone()).or_insert(Home { engine: mv.from, model: mv.was_model.clone() });
            record.switched.entry(mv.member.clone()).or_default().push(unix_now());
        }
        let setup = self.engines_of(squad).proposals.iter().any(|p| p.id == proposal && p.why == Why::Setup);
        if setup {
            record.homes.insert(mv.member.clone(), Home { engine: mv.to, model: mv.model.clone() });
        }
        self.save_engines_record(squad, &record);
        let updated = self.engines_of(squad).proposals.iter_mut().find(|p| p.id == proposal).and_then(|p| {
            let m = p.moves.iter_mut().find(|m| m.member == mv.member)?;
            m.done = true;
            Some(p.clone())
        });
        if let Some(proposal) = updated {
            self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
        }
    }

    /// Move member `id` as `due` says. In place: its model and effort, as
    /// `/model` and `/effort` set them. To the other engine: its ticket handed
    /// over (a WIP commit on its branch), the member closed as `/close` does and
    /// started again on the other engine, under the same name, as `/add` does —
    /// its first turn opening with equipe's brief. If the other engine can't
    /// start it, it starts again where it was. Whether it moved.
    async fn switch_member(&mut self, squad: &str, id: SessionId, due: &Due) -> bool {
        let mv = &due.mv;
        let Some(s) = self.sessions.get(&id) else { return false };
        let (name, role, access, mode, briefing, effort) = (mv.member.clone(), mv.role.clone(), s.access, s.mode, s.briefing.clone(), s.effort.clone());
        if mv.in_place() {
            if let Some(model) = mv.model.clone().filter(|m| s.model.as_deref() != Some(m.as_str())) {
                Box::pin(self.handle(Command::SetModel { session: id, model })).await;
            }
            if mv.effort.is_some() && mv.effort != effort {
                Box::pin(self.handle(Command::SetEffort { session: id, effort: mv.effort.clone() })).await;
            }
            self.log_engines(squad, format!("{name}: model {:?}, effort {:?}", mv.model, mv.effort));
            return true;
        }
        let model = match (mv.to, mv.model.clone()) {
            (Engine::Codex, Some(hint)) => self.codex_model(&hint).await,
            (_, model) => model,
        };
        let brief = self.hand_over(squad, id, due);
        let fresh = equipe_protocol::new_session_id();
        self.engines.names.insert(fresh, name.clone());
        self.engines.briefs.insert(fresh, brief);
        // What `/close` sends, then what `/add` does — under the same name.
        Box::pin(self.handle(Command::Close { session: id })).await;
        let spec = SessionSpec { engine: Some(mv.to), model, access, cwd: squad_home::path(&self.config.home, squad), role: Some(role.clone()), squad: Some(squad.to_owned()) };
        Box::pin(self.handle(Command::Start { session: fresh, spec })).await;
        let moved = self.sessions.contains_key(&fresh);
        let now_id = if moved {
            fresh
        } else {
            // The other engine couldn't take it: back where it was.
            self.engines.names.remove(&fresh);
            let brief = self.engines.briefs.remove(&fresh);
            let again = equipe_protocol::new_session_id();
            self.engines.names.insert(again, name.clone());
            if let Some(brief) = brief {
                self.engines.briefs.insert(again, brief);
            }
            let spec = SessionSpec { engine: Some(mv.from), model: mv.was_model.clone(), access, cwd: squad_home::path(&self.config.home, squad), role: Some(role), squad: Some(squad.to_owned()) };
            Box::pin(self.handle(Command::Start { session: again, spec })).await;
            let text = say!(self, "engines.news_move_failed", name = name, from = engine_name(mv.from), to = engine_name(mv.to));
            self.log_engines(squad, format!("{name}: couldn't start on {}; back on {}", engine_name(mv.to), engine_name(mv.from)));
            self.brief_pm(squad, format!("equipe: {text}"), Some(text));
            again
        };
        if let Some(session) = self.sessions.get_mut(&now_id) {
            session.briefing = briefing;
        }
        if moved {
            if mv.effort.is_some() {
                Box::pin(self.handle(Command::SetEffort { session: now_id, effort: mv.effort.clone() })).await;
            }
            let text = say!(self, "engines.notice_moved", from = engine_name(mv.from), to = engine_name(mv.to));
            self.publish(Event::Notice { session: now_id, text });
            self.log_engines(squad, format!("{name}: {} → {} ({:?})", engine_name(mv.from), engine_name(mv.to), due.why));
        }
        if mode.is_some() {
            self.set_mode(squad, Some(name), mode);
        }
        moved
    }

    /// The model for Codex: `hint` as the account's catalog has it.
    async fn codex_model(&mut self, hint: &str) -> Option<String> {
        let cred = codex_credentials(&self.client, &self.config.store, self.config.lang).await.ok()?;
        let asked = codex::list_models(&self.client, &cred, &self.config.client_version);
        let catalog = tokio::time::timeout(SERVER_WAIT, asked).await.ok()?.ok()?;
        rules::codex_model(hint, &catalog)
    }

    /// Before member `id` moves: its ticket, if it is on one, committed as it
    /// stands and back on the board for it under its own name; and the brief its
    /// next session opens with.
    fn hand_over(&mut self, squad: &str, id: SessionId, due: &Due) -> String {
        let Some(s) = self.sessions.get_mut(&id) else { return String::new() };
        let talk = brief_talk(s.transcript.entries());
        let work = s.work.take();
        let name = due.mv.member.clone();
        let mut ticket_part = None;
        if let Some(work) = work
            && let Some(mut ticket) = self.journal(squad).and_then(|j| j.ticket(work.ticket).ok().flatten())
        {
            let mut stat = String::new();
            if let Some(sandbox) = &work.sandbox {
                let message = format!("equipe: handoff wip — T-{} {}\n\n{name} moved from {} to {}.", ticket.id, ticket.title, engine_name(due.mv.from), engine_name(due.mv.to));
                let _ = sandbox.commit_all(&message);
                stat = sandbox.diff_stat().unwrap_or_default();
            }
            ticket.status = TicketStatus::Todo;
            ticket.assignee = None;
            ticket.owner = Some(name.clone());
            let branch = ticket.branch.clone().unwrap_or_default();
            ticket.note = Some(format!("Handed over from {} to {} as {name}; the work so far is on {branch}.", engine_name(due.mv.from), engine_name(due.mv.to)));
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&ticket);
                let _ = journal.ticket_event(ticket.id, "equipe", "handed over", &format!("{} → {}", engine_name(due.mv.from), engine_name(due.mv.to)));
            }
            ticket_part = Some(brief_ticket(&ticket, &stat));
            let ticket_id = ticket.id;
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
            self.publish(Event::WorkEnded { session: id, ticket: ticket_id });
        }
        brief(&name, &due.mv, due.why, ticket_part.as_deref(), &talk)
    }
}

/// The ticket part of a brief: what it asks, where it stands, and the work's diff.
fn brief_ticket(ticket: &Ticket, stat: &str) -> String {
    let mut out = format!("## The ticket: T-{} {}\n{}\n", ticket.id, ticket.title, clip(&ticket.body, BRIEF_BODY));
    if let Some(note) = ticket.note.as_deref().filter(|n| !n.is_empty()) {
        out.push_str(&format!("\nLatest note: {}\n", clip(note, 600)));
    }
    let open: Vec<&String> = ticket.asks.iter().chain(&ticket.remaining).collect();
    if !open.is_empty() {
        out.push_str("\nStill open with the person:\n");
        for ask in open {
            out.push_str(&format!("- {ask}\n"));
        }
    }
    if let Some(branch) = &ticket.branch {
        out.push_str(&format!("\n## The work so far\nBranch {branch}, committed as it stood (\"equipe: handoff wip\").\n"));
        if !stat.trim().is_empty() {
            out.push_str(&format!("```\n{}\n```\n", clip(stat.trim(), 1_500)));
        }
    }
    out
}

/// The end of the conversation, the newest last, as much as fits.
fn brief_talk(entries: &[Entry]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut size = 0;
    for entry in entries.iter().rev() {
        let line = match entry {
            Entry::User { text } => format!("Person: {}", clip(text, 1_200)),
            Entry::Reply { text } => format!("You: {}", clip(text, 1_800)),
            Entry::Note { text } => format!("(equipe: {})", clip(text, 300)),
            _ => continue,
        };
        if size + line.len() > BRIEF_TALK && !lines.is_empty() {
            break;
        }
        size += line.len();
        lines.push(line);
    }
    lines.reverse();
    lines.join("\n\n")
}

/// What a member's first turn on the other engine opens with: who it is, why
/// it moved, the ticket and its work, the end of its conversation, what next.
fn brief(name: &str, mv: &Move, why: Why, ticket: Option<&str>, talk: &str) -> String {
    let because = match why {
        Why::Setup => "the person set the squad up to use both engines".to_owned(),
        Why::Return => format!("{}'s plan has room again", engine_name(mv.to)),
        Why::Exhausted => format!("{}'s plan ran out until its reset", engine_name(mv.from)),
        _ => format!("{}'s plan is running low", engine_name(mv.from)),
    };
    let mut out = format!(
        "[equipe] You are {name}, carrying on on {} what you were doing on {} — {because}. This conversation is new: \
here is where things stand. Read the repository's AGENTS.md, carry on from the state below, and don't redo what is done.\n\n",
        engine_name(mv.to),
        engine_name(mv.from)
    );
    if let Some(ticket) = ticket {
        out.push_str(ticket);
        out.push('\n');
    }
    if !talk.trim().is_empty() {
        out.push_str("## Where the conversation was\n");
        out.push_str(talk);
        out.push_str("\n\n");
    }
    out.push_str("## Next\n");
    out.push_str(if ticket.is_some() {
        "Pick the ticket up where it stopped: check the branch's diff against the ticket, finish what is left, and hand it in as usual. \
Questions still open with the person stay open — don't ask them again."
    } else {
        "Nothing is in progress: carry on with the team as before."
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend_move() -> Move {
        Move { member: "backend".into(), role: "backend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("opus".into()), done: false }
    }

    #[test]
    fn a_brief_says_who_why_the_ticket_the_work_and_what_next() {
        let ticket = Ticket {
            id: 42,
            title: "Rate limit the login".into(),
            body: "Five tries a minute per address.".into(),
            status: TicketStatus::Todo,
            kind: TicketKind::Code,
            owner: Some("backend".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: Some("equipe/backend-t42".into()),
            worktree: None,
            merged: None,
            off_main: None,
            created_by: "pm".into(),
            note: None,
            asks: vec!["Which store for counters?".into()],
            remaining: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            images: Default::default(),
        };
        let part = brief_ticket(&ticket, " src/login.rs | 12 ++++++");
        let talk = brief_talk(&[Entry::User { text: "Use Redis".into() }, Entry::Reply { text: "Counting in Redis now; tests next.".into() }]);
        let text = brief("backend", &backend_move(), Why::Shift, Some(&part), &talk);
        for wanted in ["You are backend", "on Codex", "on Claude", "Claude's plan is running low", "T-42 Rate limit the login", "Five tries", "Which store for counters?", "equipe/backend-t42", "src/login.rs", "Person: Use Redis", "You: Counting in Redis", "## Next", "AGENTS.md"] {
            assert!(text.contains(wanted), "{wanted} in:\n{text}");
        }
        assert!(text.find("Person: Use Redis") < text.find("You: Counting"), "oldest first");
        // At the desk: no ticket.
        let desk = brief("qa", &Move { member: "qa".into(), role: "qa".into(), ..backend_move() }, Why::Return, None, "");
        assert!(desk.contains("Codex's plan has room again") && desk.contains("Nothing is in progress") && !desk.contains("## The ticket"));
    }

    #[test]
    fn a_brief_keeps_to_its_size_with_the_newest_of_a_long_conversation() {
        let entries: Vec<Entry> = (0..200).map(|i| Entry::Reply { text: format!("step {i} {}", "x".repeat(400)) }).collect();
        let talk = brief_talk(&entries);
        assert!(talk.len() <= BRIEF_TALK + 2_000, "{}", talk.len());
        assert!(talk.contains("step 199") && !talk.contains("step 0 "));
        let long = Ticket { body: "y".repeat(20_000), ..Ticket { id: 1, title: "t".into(), body: String::new(), status: TicketStatus::Todo, kind: TicketKind::Code, owner: None, assignee: None, project: None, depends_on: vec![], branch: None, worktree: None, merged: None, off_main: None, created_by: "you".into(), note: None, asks: vec![], remaining: vec![], priority: Default::default(), rank: 0, updated_at: None, images: Default::default() } };
        let text = brief("backend", &backend_move(), Why::Drain, Some(&brief_ticket(&long, "")), &talk);
        // About 2–4k tokens: well under 16k characters.
        assert!(text.len() < 16_000, "{}", text.len());
    }
}
