//! Both plans, watched; members moved between engines at their ticket
//! boundaries. Each engine's usage comes in as [`Event::PlanUsage`] and is
//! kept with when it was read; the squad's rules (`crate::switching`) say what
//! to tell, hold and move, and this makes it so:
//!
//! - Claude's usage comes with its turns: told when it changes, and told again
//!   as still holding while a turn keeps answering ([`ToldPlan`]). When members
//!   should move toward an engine nobody read lately, it is read then: Claude
//!   with one tiny turn of the small model, Codex through its CLI;
//! - in `suggest`, a card the person answers ([`Command::ApplyEngineProposal`]);
//!   in `auto`, the moves are made and the card tells of them, with an undo;
//! - a move waits for its member's boundary: idle between tickets, or before
//!   it picks up the next one ([`Runtime::dispatch`] asks here first) — never
//!   mid-turn. A move made `now` (the person's "Move now", a plan draining or
//!   out) happens at the end of the member's current turn even mid-ticket, at
//!   a clean point only: nothing running, queued or waiting on the person;
//! - moving is what `/close` and `/add` do ([`Command::Close`], then
//!   [`Command::Start`] on the other engine, with the same name), after a WIP
//!   commit on the ticket's branch — its worktree kept, the new session
//!   picking the ticket up there — and the new session's first message is a
//!   hand-over packet equipe writes ([`packet`]): the ticket and its plan,
//!   what is done (the diff), in progress, next and open, what the person said
//!   verbatim, the last commands and their results, what waits on the person,
//!   the end of the conversation verbatim, and what to check before editing.
//!   The old conversation stays in the journal, named on the ticket's record;
//! - a member the person put on an engine themselves is pinned there: the
//!   rules move it only off an engine that ran out, and back.
//!
//! Every decision goes to the squad's journal and to the PM's conversation as
//! news. Nothing here spends credits or overage: an engine out of its plan is
//! waited for or left, never paid for.

use equipe_protocol::engines::{HandOver, Move, Proposal, Reviewed, Switching, Why, Window};
use equipe_protocol::{EnginePlan, PlanUsage};
use serde::{Deserialize, Serialize};

use super::*;
use crate::switching::{self as rules, Hold, Level, Mate, Memory, Reach, Reading};

/// How long a look at whether Claude Code is signed in holds.
const READY_FOR: Duration = Duration::from_secs(10 * 60);
/// How often, at most, the Claude plan's usage is read on its own: when
/// members should move toward Claude and no turn told it lately.
const CLAUDE_USAGE_EVERY: Duration = Duration::from_secs(15 * 60);
/// The decisions the journal keeps, newest last.
const LOG_KEPT: usize = 200;
/// How much of the conversation a packet carries verbatim: the last messages,
/// at most this many characters (~5k tokens), each message at most `BRIEF_LINE`.
const BRIEF_TALK: usize = 20_000;
const BRIEF_MESSAGES: usize = 20;
const BRIEF_LINE: usize = 2_500;
const BRIEF_BODY: usize = 6_000;
/// A branch's diff goes in whole up to this size; beyond, its stat alone.
const BRIEF_DIFF: usize = 12_000;
/// What the person said on the ticket, verbatim, at most.
const BRIEF_PERSON: usize = 5_000;
/// The last commands a packet shows, with their results.
const BRIEF_COMMANDS: usize = 8;

/// Claude Code tells its plan's usage when it changes: a turn that keeps
/// answering says its last reading still holds, this often.
const CLAUDE_PLAN_HOLDS: Duration = Duration::from_secs(5 * 60);

/// The Claude plan's usage a turn last told. Claude Code tells it at the
/// turn's first answer and again only when it changes, so while the turn's
/// requests keep coming back, what it told is still the usage now — not a
/// reading going stale.
#[derive(Default)]
pub(super) struct ToldPlan {
    told: Option<(Activity, std::time::Instant)>,
}

impl ToldPlan {
    /// Something the turn said at `now`: its plan, kept; a request's answer,
    /// the plan told again once [`CLAUDE_PLAN_HOLDS`] has passed since.
    pub(super) fn heard(&mut self, activity: &Activity, now: std::time::Instant) -> Option<Activity> {
        match activity {
            Activity::Plan { .. } => {
                self.told = Some((activity.clone(), now));
                None
            }
            Activity::Output(_) => {
                let (plan, at) = self.told.as_mut()?;
                if now.duration_since(*at) < CLAUDE_PLAN_HOLDS {
                    return None;
                }
                *at = now;
                Some(plan.clone())
            }
            _ => None,
        }
    }
}

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
    /// When the Claude plan's usage was last read on its own (or tried), and
    /// whether a read is under way.
    claude_read: Option<std::time::Instant>,
    claude_reading: bool,
    /// Each plan's week as the Usage screen reads it: its pace, and who used it.
    loads: HashMap<Engine, crate::plan_use::Load>,
    /// Where each engine stood when the screens last asked (signed in,
    /// out, not installed): who reviews the designer's work goes by it.
    states: HashMap<Engine, equipe_protocol::EngineState>,
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
    /// Holds the person was told keep tickets waiting, as (engine, until).
    told_held: Vec<(Engine, i64)>,
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
    /// The members the person put on an engine themselves: they stay there.
    #[serde(default)]
    pins: HashMap<String, Engine>,
    /// Every decision, newest last: when, and what.
    #[serde(default)]
    log: Vec<(i64, String)>,
    /// The moves to the other engine, newest last: which session each member
    /// carries on from, for the screens to show it as one member.
    #[serde(default)]
    moves: Vec<HandOver>,
}

/// The moves the journal keeps, with their packets.
const MOVES_KEPT: usize = 100;

/// `text` cut to [`equipe_protocol::engines::PACKET_MOST`] bytes, on a character boundary.
fn packet_kept(text: &str) -> &str {
    let mut end = text.len().min(equipe_protocol::engines::PACKET_MOST);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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

    /// Whether a move to another engine waits for member `name`: it starts
    /// no new run meanwhile, so its runs end and the move comes.
    pub(super) fn moving(&self, squad: &str, name: &str) -> bool {
        self.waits_for(squad, name)
    }

    /// Whether members on `engine` take no new ticket now.
    pub(super) fn held(&self, squad: &str, engine: Engine) -> bool {
        self.hold_on(squad, engine).is_some()
    }

    /// The hold on `engine`, if any: until when.
    fn hold_on(&self, squad: &str, engine: Engine) -> Option<i64> {
        let now = unix_now();
        self.squads.get(squad)?.holds.iter().filter(|h| h.engine == engine && h.until > now).map(|h| h.until).max()
    }

    /// The engines as just looked at.
    pub(super) fn saw(&mut self, statuses: &[equipe_protocol::EngineStatus]) {
        for status in statuses {
            self.states.insert(status.engine, status.state);
        }
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

    // ── who reviews the designer's work ──

    /// Who reviews the design work of a member of `squad` on `engine`, at
    /// the turn starting now (`crate::design::reviewer`): the squad's
    /// setting, and the other engine as it stands — signed in as the screens
    /// last saw it (a member running on it says so too; never looked at, it
    /// is there when its program is), its plan as last read and burning.
    pub(super) fn design_reviewer(&self, squad: &str, engine: Engine) -> Reviewed {
        use crate::design::reviewer::{self, Limit, Standing};
        use equipe_protocol::EngineState;
        let setting = self.journals.get(squad).map(Journal::design_reviewer).unwrap_or_default();
        let other = reviewer::across(engine);
        let in_use = self.sessions.values().any(|s| s.engine == other);
        let here = || {
            let binary = match other {
                Engine::Claude => &self.config.claude_binary,
                Engine::Codex => &self.config.codex_binary,
            };
            codex_app::find(binary).is_some()
        };
        let now = unix_now();
        let fallback = Memory::default();
        let memory = self.engines.squads.get(squad).map_or(&fallback, |s| &s.memory);
        let limits = || {
            let read = self.readings().into_iter().find(|r| r.engine == other);
            read.map(|r| rules::windows_at(&r, memory, now).into_iter().map(|(window, used, runs_out)| Limit { window, used, runs_out }).collect()).unwrap_or_default()
        };
        let standing = match self.engines.states.get(&other) {
            _ if in_use => Standing::Ready(limits()),
            Some(EngineState::SignedIn) => Standing::Ready(limits()),
            Some(EngineState::SignedOut) => Standing::SignedOut,
            Some(EngineState::NotInstalled) => Standing::NotInstalled,
            Some(EngineState::Unknown) | None if here() => Standing::Ready(limits()),
            Some(EngineState::Unknown) | None => Standing::NotInstalled,
        };
        reviewer::choose(setting, engine, &standing)
    }

    // ── readings ──

    /// Each read week's pace and who used it, as the minute's pass found them.
    pub(super) fn engine_loads(&mut self, loads: Vec<(Engine, crate::plan_use::Load)>) {
        self.engines.loads = loads.into_iter().collect();
    }

    /// The engines' latest readings, each with its week's pace when known.
    fn readings(&self) -> Vec<Reading> {
        self.engines
            .plans
            .values()
            .map(|plan| {
                let mut r = Reading::of(plan);
                r.pace = self.engines.loads.get(&plan.engine).and_then(|l| l.pace);
                r
            })
            .collect()
    }

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
        let now = unix_now();
        let mut team: Vec<(u64, Mate)> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad) && s.is_member_itself())
            .filter_map(|s| {
                let m = s.member.as_ref()?;
                let home = record.homes.get(&m.name);
                let pin = record.pins.get(&m.name).copied();
                let load_bp = self.engines.loads.get(&s.engine).map(|l| l.members.iter().find(|(sq, name, _)| sq == squad && *name == m.name).map_or(0, |(_, _, bp)| *bp));
                let ticket_left = s.work.as_ref().and(s.estimate.as_ref()).and_then(|e| e.done_at).map(|at| at as i64 - now);
                Some((
                    s.joined,
                    Mate {
                        name: m.name.clone(),
                        role: m.role.clone(),
                        engine: s.engine,
                        home: pin.or(home.map(|h| h.engine)).unwrap_or(s.engine),
                        home_model: home.map_or_else(|| s.model.clone(), |h| h.model.clone()),
                        model: s.model.clone(),
                        effort: s.effort.clone(),
                        edits: s.edits(),
                        pinned: pin.is_some(),
                        switched: record.switched.get(&m.name).cloned().unwrap_or_default(),
                        waiting: self.engines.waits_for(squad, &m.name),
                        load_bp,
                        ticket_left,
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
        let readings = self.readings();
        let now = unix_now();
        let decision = {
            let memory = &mut self.engines_of(squad).memory;
            rules::decide(&rules::Squad { readings: &readings, mates: &mates, reach: &reach, mode, now }, memory)
        };
        self.engines_of(squad).holds = decision.hold.clone();
        // Where members would go, not read lately: read it now.
        for engine in decision.refresh {
            let reading = match engine {
                Engine::Claude => self.read_claude_usage(),
                Engine::Codex => self.read_codex_usage(true),
            };
            if reading {
                crate::daemon_log!("equipe: engines ({squad}): {} usage is stale or unknown; reading it before anything moves toward it", engine_name(engine));
            }
        }
        for notice in decision.notices {
            self.tell_level(squad, notice);
        }
        if let Some(batch) = decision.batch {
            self.propose_batch(squad, mode, batch);
        }
    }

    /// Tickets a member would have started wait for `engine`'s hold: the
    /// person and the PM are told, once per hold, which and until when — work
    /// that stops is never silent.
    pub(super) fn tell_held(&mut self, squad: &str, engine: Engine, tickets: &[TicketId]) {
        let Some(until) = self.engines.hold_on(squad, engine) else { return };
        let told = &mut self.engines_of(squad).told_held;
        if tickets.is_empty() || told.contains(&(engine, until)) {
            return;
        }
        told.push((engine, until));
        let name = engine_name(engine);
        let list = tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        let span = self.span(until - unix_now());
        // A card already offers to move them: the person is pointed at it, one tap away.
        let offered = self.engines.proposals(squad).iter().any(|p| !p.applied && p.engine == Some(engine) && !p.moves.is_empty());
        let text = if offered {
            say!(self, "engines.news_held_offer", tickets = list, engine = name, span = span)
        } else {
            say!(self, "engines.news_held", tickets = list, engine = name, span = span)
        };
        self.log_engines(squad, format!("held: {list} wait for {name} ({span})"));
        let next = if offered {
            "A card on their screen already offers to move those members to the other engine: tell them it is one tap."
        } else {
            "They may top up or reset the plan, or move members to another engine."
        };
        let line = format!(
            "equipe: {list} are waiting and won't start: {name} reached its plan's limit, and equipe starts nothing new on it until it resets in {span}. Tell the person plainly; don't requeue them or change models to get around it. {next}"
        );
        self.brief_pm(squad, line, Some(text));
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

    /// "35m", "2h", "3d" (in the person's language): how long until then.
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
        // A lighter model trades quality for room: only the person makes that call.
        let auto = mode == Switching::Auto && batch.why != Why::StepDown;
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
            forecast: batch.forecast,
        };
        let text = self.proposal_words(squad, &proposal);
        self.log_engines(squad, format!("{} {:?} ({}): {}", if auto { "made" } else { "proposed" }, proposal.why, engine_name(batch.engine), Self::names(&proposal.moves)));
        if auto {
            self.schedule(squad, &proposal);
        }
        self.engines_of(squad).proposals.push(proposal.clone());
        self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
    }

    /// What a card says, as one line for the news — with, for moves made,
    /// when each member moves, as true as the rule that moves it.
    fn proposal_words(&self, squad: &str, p: &Proposal) -> String {
        let names = Self::names(&p.moves);
        let engine = p.engine.map(engine_name).unwrap_or_default();
        let to = p.to().map(engine_name).unwrap_or(engine);
        let window = self.window_words(p.window);
        let span = self.span(p.resets_at - unix_now());
        let used = p.used_percent;
        let head = match (p.why, p.applied) {
            (Why::Setup, _) => say!(self, "engines.news_setup", names = names),
            (Why::StepDown, false) => say!(self, "engines.news_step_down_ask", names = names),
            (Why::StepDown, true) => say!(self, "engines.news_step_down", names = names),
            (Why::Return, _) => say!(self, "engines.news_return", engine = engine, window = window, used = used, names = names),
            (Why::Exhausted, false) => say!(self, "engines.news_hand_over_ask", engine = engine, span = span, names = names, to = to),
            (Why::Exhausted, true) => say!(self, "engines.news_hand_over", engine = engine, span = span, names = names, to = to),
            (_, false) => say!(self, "engines.news_shift_ask", engine = engine, window = window, used = used, span = span, names = names, to = to),
            (_, true) => say!(self, "engines.news_shift", engine = engine, window = window, used = used, names = names, to = to),
        };
        let mut out = match (p.forecast.and_then(|f| f.runs_out_at.map(|at| (at, f))), p.why) {
            (Some((at, f)), Why::Shift | Why::Drain) => {
                let now = unix_now();
                let pace = say!(self, "engines.news_pace", engine = engine, out = self.span(at - now), early = self.span(f.resets_at - at + now));
                format!("{head} · {pace}")
            }
            _ => head,
        };
        if p.applied && !matches!(p.why, Why::Setup) {
            let when = self.when_words(squad, &p.moves);
            if !when.is_empty() {
                out = format!("{out} — {when}");
            }
        }
        out
    }

    /// When each move still waiting reaches its member, as the boundary rule
    /// says it ([`controls::move_when`]): one sentence each.
    fn when_words(&self, squad: &str, moves: &[Move]) -> String {
        moves
            .iter()
            .filter(|m| !m.done && !m.in_place())
            .filter_map(|m| {
                let id = self.member_id(squad, &m.member)?;
                let s = self.sessions.get(&id)?;
                let mover = crate::controls::Mover { name: m.member.clone(), from: m.from, working: s.is_working(), on_ticket: !self.runs_of(id).is_empty(), now: m.now };
                Some(crate::controls::move_when(self.config.lang, &mover, m.to))
            })
            .collect::<Vec<_>>()
            .join(" ")
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
        let proposal = Proposal { id: equipe_protocol::new_session_id(), why: Why::Setup, engine: None, window: None, used_percent: 0, resets_at: 0, moves, applied: false, proposed_at: now_secs(), forecast: None };
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

    /// The person's yes: each move at its member's boundary — `now`, at the
    /// end of each one's turn, a ticket in progress handed over. On a card
    /// whose moves are made already, `now` hurries those still waiting.
    pub(super) async fn apply_engine_proposal(&mut self, squad: &str, id: u64, now: bool) {
        let Some(mut proposal) = self.engines_of(squad).proposals.iter().find(|p| p.id == id).cloned() else { return };
        if proposal.applied {
            if now {
                self.hurry(squad, id).await;
            }
            return;
        }
        for mv in proposal.moves.iter_mut().filter(|m| !m.in_place()) {
            mv.now |= now;
        }
        self.schedule(squad, &proposal);
        self.close_proposal(squad, id, true);
        let names = Self::names(&proposal.moves);
        self.log_engines(squad, format!("the person applied {:?}{}: {names}", proposal.why, if now { " now" } else { "" }));
        let when = self.when_words(squad, &proposal.moves);
        let text = say!(self, "engines.news_applied", names = names, when = when);
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
        self.apply_engine_moves(squad).await;
    }

    /// The person's "Move now" on moves made already: each still waiting for
    /// its member's next ticket moves at the end of its turn instead.
    async fn hurry(&mut self, squad: &str, id: u64) {
        let mine = self.engines_of(squad);
        let mut hurried = Vec::new();
        for d in mine.due.iter_mut().filter(|d| d.proposal == id && !d.mv.now && !d.mv.in_place()) {
            d.mv.now = true;
            hurried.push(d.mv.member.clone());
        }
        let updated = mine.proposals.iter_mut().find(|p| p.id == id).map(|p| {
            for m in p.moves.iter_mut().filter(|m| hurried.contains(&m.member)) {
                m.now = true;
            }
            p.clone()
        });
        if hurried.is_empty() {
            return;
        }
        if let Some(proposal) = updated {
            let when = self.when_words(squad, &proposal.moves);
            self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
            self.log_engines(squad, format!("the person moved {} now", hurried.join("·")));
            let text = say!(self, "engines.news_now", names = hurried.join("·"), when = when);
            self.brief_pm(squad, format!("equipe: {text}"), Some(text));
        }
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
        if matches!(proposal.why, Why::Setup | Why::Person) {
            let mut record = self.engines_record(squad);
            for m in proposal.moves.iter() {
                // The person's own move taken back: not pinned there any more.
                if proposal.why == Why::Person && record.pins.get(&m.member) == Some(&m.to) {
                    record.pins.remove(&m.member);
                }
                if m.done {
                    record.homes.insert(m.member.clone(), Home { engine: m.from, model: m.was_model.clone() });
                }
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

    /// The person's own move: `members` (none: all) to `engine`, each at its
    /// next boundary — `now`, at the end of its turn, mid-ticket too — told
    /// on a card that offers to undo it, on `model` when the person picked
    /// one, else the engine's recommendation. They are pinned there.
    pub(super) async fn move_engine(&mut self, squad: &str, members: Vec<String>, engine: Engine, model: Option<String>, effort: Option<String>, now: bool) {
        let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
        let effort = effort.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty());
        let moves: Vec<Move> = self
            .mates(squad)
            .into_iter()
            .filter(|m| members.is_empty() || members.iter().any(|n| n == &m.name))
            .filter(|m| m.engine != engine)
            .map(|m| Move { member: m.name, role: m.role, from: m.engine, to: engine, model: model.clone(), effort: effort.clone(), was_model: m.model, done: false, now })
            .collect();
        if moves.is_empty() {
            let text = say!(self, "engines.move_none", engine = engine_word(engine));
            return self.squad_error(squad, text);
        }
        let proposal = Proposal { id: equipe_protocol::new_session_id(), why: Why::Person, engine: None, window: None, used_percent: 0, resets_at: 0, moves, applied: true, proposed_at: now_secs(), forecast: None };
        let names = Self::names(&proposal.moves);
        self.log_engines(squad, format!("the person moved {names} to {}{}", engine_word(engine), if now { " now" } else { "" }));
        // Where the person put them: pinned there from now on.
        let mut record = self.engines_record(squad);
        for m in &proposal.moves {
            record.pins.insert(m.member.clone(), engine);
        }
        self.save_engines_record(squad, &record);
        self.engines_of(squad).proposals.push(proposal.clone());
        self.schedule(squad, &proposal);
        let when = self.when_words(squad, &proposal.moves);
        self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
        let text = say!(self, "engines.news_person", names = names, engine = engine_word(engine), when = when);
        self.brief_pm(squad, format!("equipe: {text}"), Some(text));
        self.apply_engine_moves(squad).await;
    }

    /// The person sets member `name` (on Claude Code) to a provider's
    /// `model`: it moves to Codex's agent on it at its next boundary, as the
    /// person's own move does, told on a card that offers to undo it.
    pub(super) async fn move_to_model(&mut self, squad: &str, name: String, session: SessionId, model: String) {
        let Some(mate) = self.mates(squad).into_iter().find(|m| m.name == name) else { return };
        let mv = Move { member: mate.name, role: mate.role, from: mate.engine, to: Engine::Codex, model: Some(model.clone()), effort: None, was_model: mate.model, done: false, now: false };
        let proposal = Proposal { id: equipe_protocol::new_session_id(), why: Why::Person, engine: None, window: None, used_percent: 0, resets_at: 0, moves: vec![mv], applied: true, proposed_at: now_secs(), forecast: None };
        self.log_engines(squad, format!("the person set {name} to {model}"));
        self.engines_of(squad).proposals.push(proposal.clone());
        self.schedule(squad, &proposal);
        self.publish(Event::EngineProposed { squad: squad.to_owned(), proposal });
        let (shown, provider) = self.on_provider(&model).unwrap_or_default();
        let text = say!(self, "notice.provider_move", model = shown, provider = provider);
        self.publish(Event::Notice { undo: None, session, text });
        self.apply_engine_moves(squad).await;
    }

    pub(super) fn show_engine_setup(&mut self, squad: &str) {
        let mates = self.mates(squad);
        let reach = self.reach();
        self.offer_setup(squad, &mates, &reach, true);
    }

    // ── at the boundary ──

    /// Read the Claude plan's usage off the loop, with one tiny turn of the
    /// person's Claude Code ([`claude_code::read_plan`]): no member's turn
    /// told it lately. At most every [`CLAUDE_USAGE_EVERY`]. Whether a read started.
    fn read_claude_usage(&mut self) -> bool {
        if self.engines.claude_reading || self.engines.claude_read.is_some_and(|at| at.elapsed() < CLAUDE_USAGE_EVERY) {
            return false;
        }
        self.engines.claude_reading = true;
        self.engines.claude_read = Some(std::time::Instant::now());
        let (binary, home, internal) = (self.config.claude_binary.clone(), self.config.home.clone(), self.internal.clone());
        tokio::spawn(async move {
            let cwd = if home.is_dir() { home } else { std::env::temp_dir() };
            let usage = match claude_code::read_plan(&binary, &cwd).await {
                Some(Activity::Plan { five_hour, seven_day }) => Some(super::claude_plan(five_hour, seven_day)),
                _ => None,
            };
            let _ = internal.send(super::Internal::ClaudePlan(usage));
        });
        true
    }

    /// The Claude plan's usage, as just read on its own: told the screens,
    /// weighed at once, and the moves it brings made where members are free.
    pub(super) async fn on_claude_plan(&mut self, usage: Option<PlanUsage>) {
        self.engines.claude_reading = false;
        let Some(usage) = usage else {
            crate::daemon_log!("equipe: engines: Claude's plan usage couldn't be read; tried again later");
            return;
        };
        self.publish(Event::PlanUsage { usage, engine: Engine::Claude });
        for squad in self.squads_with_members() {
            self.apply_engine_moves(&squad).await;
        }
    }

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

    /// Whether member `id` is at a boundary for `due`: a clean point — not
    /// mid-turn, nothing queued for it, no approval or question of its
    /// waiting on the person, nothing it handed in or put down still being
    /// settled; for a move to the other engine, between tickets too — unless
    /// the move is `now` (or its engine is out), when its ticket is handed over.
    fn at_boundary(&mut self, squad: &str, id: SessionId, due: &Due) -> bool {
        let Some(s) = self.sessions.get(&id) else { return false };
        if s.is_working() || !s.queued.is_empty() || !s.steering.is_empty() || self.pending.values().any(|p| p.approval.session == id) {
            return false;
        }
        if s.work.as_ref().is_some_and(|w| w.handed_in.is_some() || w.waiting.is_some()) {
            return false;
        }
        // Each of its runs at a clean point too; it starts no new one while
        // the move waits ([`Self::moving`]).
        let runs = self.runs_of(id);
        for run in &runs {
            let Some(r) = self.sessions.get(run) else { continue };
            if r.is_working() || !r.queued.is_empty() || !r.steering.is_empty() || self.pending.values().any(|p| p.approval.session == *run) {
                return false;
            }
            if r.work.as_ref().is_some_and(|w| w.handed_in.is_some() || w.waiting.is_some()) {
                return false;
            }
        }
        let on_ticket = s.work.is_some() || !runs.is_empty();
        let name = due.mv.member.clone();
        if self.question_sets(squad).iter().any(|q| q.member == name) {
            return false;
        }
        boundary(due.mv.in_place(), on_ticket, due.mv.now || due.why == Why::Exhausted)
    }

    /// Make the moves whose members are at their boundary. Asked before the
    /// board hands out work, so a member moving picks its next ticket up on
    /// the new engine.
    pub(super) async fn apply_engine_moves(&mut self, squad: &str) {
        if self.engines.applying || self.engines.squads.get(squad).is_none_or(|s| s.due.is_empty()) {
            return;
        }
        self.engines.applying = true;
        let ready = self.reach().ready;
        let mut at = 0;
        while let Some(due) = self.engines.squads.get(squad).and_then(|s| s.due.get(at)) {
            let (proposal, why, mv) = (due.proposal, due.why, due.mv.clone());
            let Some(id) = self.member_id(squad, &mv.member) else {
                // Gone: nothing to move.
                self.engines_of(squad).due.remove(at);
                continue;
            };
            // Signed out of where equipe would move it since: it stays.
            if !mv.in_place() && why != Why::Person && !ready.contains(&mv.to) {
                self.engines_of(squad).due.remove(at);
                self.log_engines(squad, format!("{}: not moved — {} isn't signed in", mv.member, engine_name(mv.to)));
                continue;
            }
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
        // The setup, or the person's own move: where it belongs from now on.
        let setup = self.engines_of(squad).proposals.iter().any(|p| p.id == proposal && matches!(p.why, Why::Setup | Why::Person));
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

    /// A move to the other engine made: kept on the squad's record with the
    /// packet the new session opens with, and told to the screens, so they
    /// show the member as one, its conversations one after the other.
    fn keep_move(&mut self, squad: &str, hand_over: HandOver, packet: &str) {
        let mut record = self.engines_record(squad);
        record.moves.push(hand_over.clone());
        let gone: Vec<SessionId> = record.moves.drain(..record.moves.len().saturating_sub(MOVES_KEPT)).map(|h| h.to_session).collect();
        self.save_engines_record(squad, &record);
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_hand_over_packet(hand_over.to_session, packet);
            for id in gone {
                let _ = journal.forget_hand_over_packet(id);
            }
        }
        self.publish(Event::MemberMoved { squad: squad.to_owned(), hand_over });
    }

    /// The squad's moves to the other engine on record, oldest first.
    pub(super) fn hand_overs(&mut self, squad: &str) -> Vec<HandOver> {
        self.engines_record(squad).moves
    }

    /// What session `session` was handed when it took over
    /// ([`Command::HandOver`]): its packet and the conversation before it, the
    /// latest lines of it — to the screens following the squad.
    pub(super) fn send_hand_over(&mut self, squad: &str, session: SessionId) {
        let Some(from) = self.engines_record(squad).moves.iter().rev().find(|h| h.to_session == session).map(|h| h.from_session) else {
            return;
        };
        let Some(journal) = self.journal(squad) else { return };
        let packet = journal.hand_over_packet(session).unwrap_or_default();
        let (mut lines, mut lines_at) = journal.conversation_at(from).unwrap_or_default();
        let cut = lines.len().saturating_sub(equipe_protocol::engines::EARLIER_LINES);
        lines.drain(..cut);
        lines_at.drain(..cut.min(lines_at.len()));
        self.publish_in(Some(squad), Event::HandOver { squad: squad.to_owned(), session, packet, lines, lines_at });
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
            // An open model (a provider's, Nebius's) is its own: not one of the account's.
            (Engine::Codex, Some(model)) if crate::nemotron::is_agent_model(&model) => Some(model),
            (Engine::Codex, Some(hint)) => self.codex_model(&hint).await,
            (_, model) => model,
        };
        let first_run = self.runs_of(id).first().copied();
        let ticket = first_run.or(Some(id)).and_then(|at| self.sessions.get(&at)).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let brief = self.hand_over(squad, id, due).await;
        let packet = packet_kept(&brief).to_owned();
        let fresh = equipe_protocol::new_session_id();
        self.engines.names.insert(fresh, name.clone());
        self.engines.briefs.insert(fresh, brief);
        // The conversations the person opened with it go with it, not with
        // the session it leaves: parked on the new one so `/close` spares them.
        let kept: Vec<SessionId> = self.sessions.iter().filter(|(_, s)| s.kept && s.lane.as_ref().is_some_and(|(of, _)| *of == id)).map(|(&l, _)| l).collect();
        self.repoint_lanes(&kept, fresh, None);
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
        let member = self.sessions.get(&now_id).map(|s| (s.engine, s.model.clone()));
        self.repoint_lanes(&kept, now_id, member);
        if self.sessions.contains_key(&now_id) {
            let to = if moved { mv.to } else { mv.from };
            let hand_over = HandOver { member: name.clone(), from_session: id, to_session: now_id, from: mv.from, to, why: due.why, at: unix_now(), ticket };
            self.keep_move(squad, hand_over, &packet);
        }
        if moved {
            if mv.effort.is_some() {
                Box::pin(self.handle(Command::SetEffort { session: now_id, effort: mv.effort.clone() })).await;
            }
            let text = say!(self, "engines.notice_moved", from = engine_name(mv.from), to = engine_name(mv.to));
            self.publish(Event::Notice { undo: None, session: now_id, text });
            self.log_engines(squad, format!("{name}: {} → {} ({:?})", engine_name(mv.from), engine_name(mv.to), due.why));
        }
        if mode.is_some() {
            self.set_mode(squad, Some(name), mode);
        }
        moved
    }

    /// Conversations the person opened with a member now belong to `to`. Given
    /// its engine and model, each one not mid-turn takes them too: it goes on
    /// in a new conversation there, opening with its lines so far.
    fn repoint_lanes(&mut self, lanes: &[SessionId], to: SessionId, member: Option<(Engine, Option<String>)>) {
        for lane in lanes {
            let Some(s) = self.sessions.get_mut(lane) else { continue };
            if let Some((of, _)) = s.lane.as_mut() {
                *of = to;
            }
            let Some((engine, model)) = member.clone() else { continue };
            if s.engine == engine || s.is_working() {
                continue;
            }
            let said: Vec<String> = s
                .transcript
                .entries()
                .iter()
                .filter_map(|e| match e {
                    Entry::User { text, .. } => Some(format!("Person: {}", clip(text, 600))),
                    Entry::Reply { text } => Some(format!("You: {}", clip(text, 600))),
                    _ => None,
                })
                .collect();
            let recent = said[said.len().saturating_sub(16)..].join("\n");
            s.engine = engine;
            s.model = model;
            s.claude_session = None;
            s.history.clear();
            if !recent.is_empty() {
                s.briefing.push(format!(
                    "equipe: this conversation moved to {} with the PM; the engine starts afresh, so here is how it went so far (latest last):\n{recent}",
                    engine_name(engine)
                ));
            }
        }
    }

    /// The model for Codex: `hint` as the account's catalog has it.
    async fn codex_model(&mut self, hint: &str) -> Option<String> {
        let cred = codex_credentials(&self.client, &self.config.store, self.config.lang).await.ok()?;
        let asked = codex::list_models(&self.client, &cred, &self.config.client_version);
        let catalog = tokio::time::timeout(SERVER_WAIT, asked).await.ok()?.ok()?;
        rules::codex_model(hint, &catalog)
    }

    /// Before member `id` moves: its ticket, if it is on one, committed as it
    /// stands (off the loop) and back on the board for it under its own name,
    /// its worktree kept for the new session to pick up; and the hand-over
    /// packet its next session opens with. The ticket's record names the old
    /// conversation, which the journal keeps.
    async fn hand_over(&mut self, squad: &str, id: SessionId, due: &Due) -> String {
        // Its runs' tickets go over too, each the same way; the first one's
        // conversation and ticket open the packet.
        let runs = self.runs_of(id);
        let Some(s) = self.sessions.get(&id) else { return String::new() };
        let told = runs.first().and_then(|r| self.sessions.get(r)).unwrap_or(s);
        let entries = told.transcript.entries().to_vec();
        let estimate = told.estimate.as_ref().and_then(|e| e.live_what.clone().or(e.what.clone()));
        let mut works: Vec<(SessionId, Work)> = Vec::new();
        for at in std::iter::once(id).chain(runs.iter().copied()) {
            if let Some(work) = self.sessions.get_mut(&at).and_then(|s| s.work.take()) {
                works.push((at, work));
            }
        }
        let name = due.mv.member.clone();
        let (from, to) = (engine_name(due.mv.from), engine_name(due.mv.to));
        let mut waiting: Vec<String> = self.pending.values().filter(|p| p.approval.session == id || runs.contains(&p.approval.session)).map(|p| p.approval.action.lines().next().unwrap_or_default().to_owned()).collect();
        waiting.extend(self.question_sets(squad).iter().filter(|q| q.member == name).flat_map(|q| q.questions.iter().map(|x| x.question.clone())));
        let mut ticket_part = None;
        for (at, work) in works {
            let Some(mut ticket) = self.journal(squad).and_then(|j| j.ticket(work.ticket).ok().flatten()) else { continue };
            let mut tree = Tree::default();
            if let Some(sandbox) = work.sandbox.clone() {
                let message = format!("equipe: handoff wip — T-{} {}\n\n{name} moved from {from} to {to}.", ticket.id, ticket.title);
                tree = tokio::task::spawn_blocking(move || Tree::of(&sandbox, &message)).await.unwrap_or_default();
            }
            let notes: Vec<String> = self
                .journal(squad)
                .and_then(|j| j.ticket_history(ticket.id).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|(by, _, _)| *by == name)
                .map(|(_, kind, text)| format!("{kind}: {}", clip(&text, 400)))
                .rev()
                .take(8)
                .collect();
            ticket.status = TicketStatus::Todo;
            ticket.assignee = None;
            ticket.owner = Some(name.clone());
            let branch = ticket.branch.clone().unwrap_or_default();
            ticket.note = Some(format!("Handed over from {from} to {to} as {name}; the work so far is on {branch}."));
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&ticket);
                // The other engine can't resume this one's conversation: the packet carries it.
                let mut kept = journal.ticket_work(ticket.id).unwrap_or_default();
                kept.engine_session = None;
                let _ = journal.set_ticket_work(ticket.id, &kept);
                // This run is over: the new session starts the next one at once.
                let _ = journal.finish_execution(ticket.id, work.execution_generation, ExecutionState::Stopped, Some("handed_over"));
                let what = format!("{from} → {to}, mid-ticket: {}; the conversation before is kept as session {at}", tree.commit.as_deref().map_or_else(|| "nothing uncommitted".to_owned(), |c| format!("work committed as {}", &c[..c.len().min(10)])));
                let _ = journal.ticket_event(ticket.id, "equipe", "handed over", &what);
            }
            if ticket_part.is_none() {
                ticket_part = Some(brief_ticket(&ticket, &tree, &notes));
            }
            let ticket_id = ticket.id;
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
            let of = self.sessions.get(&at).and_then(|s| s.run_of);
            self.publish(Event::WorkEnded { session: at, ticket: ticket_id, of });
            self.close_run(at);
        }
        let state = State { estimate, waiting };
        packet(&name, &due.mv, due.why, ticket_part.as_deref(), &entries, &state)
    }
}

/// Whether a move reaches its member now: in place, off a ticket, or `now`
/// (a move made at the end of the turn, its ticket handed over).
fn boundary(in_place: bool, on_ticket: bool, now: bool) -> bool {
    in_place || !on_ticket || now
}

/// The worktree as the member left it: committed, with its stat and — when
/// small enough to read — its diff.
#[derive(Debug, Clone, Default)]
struct Tree {
    commit: Option<String>,
    branch: String,
    path: String,
    /// Where the branch began.
    base: String,
    stat: String,
    diff: Option<String>,
    status: String,
}

impl Tree {
    /// Commit what `sandbox` holds as it stands, and read it back. Runs git: off the loop.
    fn of(sandbox: &worktree::Sandbox, message: &str) -> Tree {
        let status = std::process::Command::new("git").arg("-C").arg(&sandbox.path).args(["status", "--short"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let commit = sandbox.commit_all(message).ok().flatten();
        let stat = sandbox.diff_stat().unwrap_or_default();
        let diff = sandbox.diff().ok().filter(|d| d.len() <= BRIEF_DIFF);
        Tree { commit, branch: sandbox.branch.clone(), path: sandbox.path.display().to_string(), base: sandbox.base.clone(), stat, diff, status }
    }
}

/// Where the member was, beyond its ticket: what it said it was doing, and
/// what of it waits on the person.
#[derive(Debug, Clone, Default)]
struct State {
    estimate: Option<String>,
    waiting: Vec<String>,
}

/// The ticket part of a packet: what it asks and its plan, what is open with
/// the person, the work as it stands — branch, worktree, the commit made of
/// it, its stat and (when small) its diff — and the member's own notes on it.
fn brief_ticket(ticket: &Ticket, tree: &Tree, notes: &[String]) -> String {
    let mut out = format!("## The ticket: T-{} {}\n{}\n", ticket.id, ticket.title, clip(&ticket.body, BRIEF_BODY));
    if let Some(plan) = ticket.plan.as_deref().filter(|p| !p.is_empty()) {
        out.push_str(&format!("\nIts plan: {plan} — read it again before you go on.\n"));
    }
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
    if let Some(branch) = ticket.branch.as_deref().or(Some(tree.branch.as_str()).filter(|b| !b.is_empty())) {
        out.push_str(&format!("\n## The work so far\nBranch {branch}"));
        if !tree.path.is_empty() {
            out.push_str(&format!(", in the worktree {} — the same one, yours to carry on in", tree.path));
        }
        out.push_str(".\n");
        match &tree.commit {
            Some(commit) => out.push_str(&format!("Everything it held was committed as it stood (\"equipe: handoff wip\", {}).\n", &commit[..commit.len().min(10)])),
            None => out.push_str("Nothing was left uncommitted.\n"),
        }
        if !tree.status.trim().is_empty() {
            out.push_str(&format!("Uncommitted when it stopped (now in that commit):\n```\n{}\n```\n", clip(tree.status.trim(), 1_500)));
        }
        if !tree.stat.trim().is_empty() {
            out.push_str(&format!("Changed since the branch began (`git diff --stat {}..{branch}`):\n```\n{}\n```\n", short(&tree.base), clip(tree.stat.trim(), 1_500)));
        }
        match &tree.diff {
            Some(diff) if !diff.trim().is_empty() => out.push_str(&format!("The diff itself:\n```diff\n{}\n```\n", diff.trim_end())),
            _ if !tree.stat.trim().is_empty() => out.push_str("The diff is too long to put here: read it with `git diff` against where the branch began.\n"),
            _ => {}
        }
    }
    if !notes.is_empty() {
        out.push_str("\n## Your own notes on this ticket, newest first\n");
        for note in notes {
            out.push_str(&format!("- {note}\n"));
        }
    }
    out
}

/// A commit's short form.
fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

/// The entries since the conversation last turned to a ticket or a project
/// (its last `Thread`): this ticket's part of it.
fn this_part(entries: &[Entry]) -> &[Entry] {
    let from = entries.iter().rposition(|e| matches!(e, Entry::Thread { .. })).map_or(0, |at| at + 1);
    &entries[from..]
}

/// What the person said, verbatim, newest last, as much as fits.
fn person_said(entries: &[Entry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut size = 0;
    for entry in entries.iter().rev() {
        let Entry::User { text, kept } = entry else { continue };
        let Some(words) = equipe_protocol::harness::words(text) else { continue };
        let line = clip(&equipe_protocol::tags::read_back(&words, kept), 1_500);
        if size + line.len() > BRIEF_PERSON && !out.is_empty() {
            break;
        }
        size += line.len();
        out.push(line);
    }
    out.reverse();
    out
}

/// The last commands and what they printed, newest last.
fn commands(entries: &[Entry]) -> Vec<String> {
    let mut out: Vec<String> = entries
        .iter()
        .rev()
        .filter_map(|e| {
            let Entry::Tool { tool, target, outcome: Some(outcome), .. } = e else { return None };
            if outcome.diff.is_some() || (outcome.output.trim().is_empty() && outcome.exit.is_none()) {
                return None;
            }
            let what = target.as_deref().map_or_else(|| tool.clone(), |t| format!("{tool} {}", clip(t, 200)));
            let how = match (outcome.ok, outcome.exit) {
                (true, _) => "ok".to_owned(),
                (false, Some(code)) => format!("failed, exit {code}"),
                (false, None) => "failed".to_owned(),
            };
            let printed = outcome.output.trim();
            Some(if printed.is_empty() { format!("- `{what}` → {how}") } else { format!("- `{what}` → {how}\n```\n{}\n```", clip(printed, 800)) })
        })
        .take(BRIEF_COMMANDS)
        .collect();
    out.reverse();
    out
}

/// The end of the conversation, verbatim, the newest last: its last
/// [`BRIEF_MESSAGES`] messages, as much as [`BRIEF_TALK`] holds.
fn brief_talk(entries: &[Entry]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut size = 0;
    for entry in entries.iter().rev() {
        let line = match entry {
            Entry::User { text, kept } => match equipe_protocol::harness::words(text) {
                Some(text) => format!("Person: {}", clip(&equipe_protocol::tags::read_back(&text, kept), BRIEF_LINE)),
                None => continue,
            },
            Entry::Reply { text } => format!("You: {}", clip(text, BRIEF_LINE)),
            Entry::Note { text, .. } => format!("(equipe: {})", clip(text, 300)),
            _ => continue,
        };
        if lines.len() >= BRIEF_MESSAGES || (size + line.len() > BRIEF_TALK && !lines.is_empty()) {
            break;
        }
        size += line.len();
        lines.push(line);
    }
    lines.reverse();
    lines.join("\n\n")
}

/// What a member's first turn on the other engine opens with — the
/// hand-over packet: who it is and why it moved; the ticket, its plan and
/// its work (on a ticket); where things stand — done, in progress, next,
/// open; what the person said, verbatim; the last commands and their
/// results; the end of the conversation, verbatim; and what to check before
/// it changes anything, so nothing is redone or lost.
fn packet(name: &str, mv: &Move, why: Why, ticket: Option<&str>, entries: &[Entry], state: &State) -> String {
    let because = match why {
        Why::Setup => "the person set the squad up to use both engines".to_owned(),
        Why::Person => "the person moved you".to_owned(),
        Why::Return => format!("{}'s plan has room again", engine_name(mv.to)),
        Why::Exhausted => format!("{}'s plan ran out until its reset", engine_name(mv.from)),
        _ => format!("{}'s plan is running low", engine_name(mv.from)),
    };
    let mut out = format!(
        "[equipe] You are {name}, carrying on on {} what you were doing on {} — {because}. This conversation is new, but nothing \
was lost: this is the hand-over packet. Read all of it before you do anything, and the repository's AGENTS.md.\n\n",
        engine_name(mv.to),
        engine_name(mv.from)
    );
    if let Some(ticket) = ticket {
        out.push_str(ticket);
        out.push('\n');
    }
    let part = if ticket.is_some() { this_part(entries) } else { entries };
    let last_reply = part.iter().rev().find_map(|e| if let Entry::Reply { text } = e { Some(text.as_str()) } else { None });
    out.push_str("## Where things stand\n");
    out.push_str(if ticket.is_some() { "- Done: what the commit and diff above hold — check them against the ticket.\n" } else { "- Done: what the conversation below says.\n" });
    if let Some(what) = state.estimate.as_deref().filter(|w| !w.trim().is_empty()) {
        out.push_str(&format!("- In progress, as you last said: {what}\n"));
    }
    match last_reply {
        Some(reply) => out.push_str(&format!("- Next: where your last reply left it — \"{}\"\n", clip(reply.trim(), 600))),
        None => out.push_str("- Next: nothing was said yet; start from the ticket.\n"),
    }
    if state.waiting.is_empty() {
        out.push_str("- Open decisions: none waiting on the person.\n");
    } else {
        out.push_str("- Open decisions, waiting on the person — don't ask them again:\n");
        for w in &state.waiting {
            out.push_str(&format!("  - {}\n", clip(w, 300)));
        }
    }
    let said = person_said(part);
    if !said.is_empty() {
        out.push_str(if ticket.is_some() { "\n## What the person said on this ticket, verbatim\n" } else { "\n## What the person said, verbatim\n" });
        for line in said {
            out.push_str(&format!("> {}\n", line.replace('\n', "\n> ")));
        }
    }
    let ran = commands(part);
    if !ran.is_empty() {
        out.push_str("\n## The last commands you ran, and what they said\n");
        out.push_str(&ran.join("\n"));
        out.push('\n');
    }
    let talk = brief_talk(entries);
    if !talk.trim().is_empty() {
        out.push_str("\n## The end of the conversation, verbatim\n");
        out.push_str(&talk);
        out.push_str("\n\n");
    }
    out.push_str("## Before you change anything\n");
    out.push_str(if ticket.is_some() {
        "1. Read this packet through.\n\
2. Check the worktree yourself: `git status`, `git log --oneline -5` and the branch's diff; run the ticket's tests and see what passes.\n\
3. Say in one line what you will do next — then do it.\n\
Carry on from that state: don't redo what is done, don't undo it, don't start a new branch or worktree. Questions still open with \
the person stay open — don't ask them again. When the ticket is done, hand it in as usual."
    } else {
        "Nothing of a ticket is in progress: carry on the conversation where it was — the next message follows it. Don't repeat what \
was said, and don't ask again what is waiting on the person."
    });
    out
}

/// An engine as the person reads its name.
fn engine_word(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend_move() -> Move {
        Move { member: "backend".into(), role: "backend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("opus".into()), done: false, now: true }
    }

    fn ticket(body: &str) -> Ticket {
        Ticket {
            id: 42,
            title: "Rate limit the login".into(),
            body: body.into(),
            status: TicketStatus::Todo,
            kind: TicketKind::Code,
            owner: Some("backend".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: Some("equipe/backend-t42".into()),
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: Some("docs/plans/login.md".into()),
            created_by: "pm".into(),
            note: None,
            asks: vec!["Which store for counters?".into()],
            remaining: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            images: Default::default(),
        }
    }

    fn tool(target: &str, ok: bool, output: &str) -> Entry {
        let outcome = equipe_protocol::ToolOutcome { ok, exit: Some(if ok { 0 } else { 1 }), ms: None, output: output.into(), skipped: 0, diff: None };
        Entry::Tool { tool: "Bash".into(), target: Some(target.into()), outcome: Some(Box::new(outcome)), label: None, background: false }
    }

    #[test]
    fn a_hand_over_packet_carries_the_ticket_its_work_the_persons_words_the_commands_and_what_to_check_first() {
        let tree = Tree {
            commit: Some("0123456789abcdef".into()),
            branch: "equipe/backend-t42".into(),
            path: "/w/backend-t42".into(),
            base: "fedcba9876543210".into(),
            stat: " src/login.rs | 12 ++++++".into(),
            diff: Some("diff --git a/src/login.rs b/src/login.rs\n+let tries = 5;".into()),
            status: " M src/login.rs".into(),
        };
        let part = brief_ticket(&ticket("Five tries a minute per address.\n\nAcceptance criteria:\n- tests pass"), &tree, &["estimate: counting in Redis".into()]);
        let entries = [
            Entry::User { text: "Earlier, at the desk: hello".into(), kept: Vec::new() },
            Entry::Thread { project: Some("api".into()) },
            Entry::User { text: "Use Redis".into(), kept: Vec::new() },
            tool("cargo test", false, "test login::five ... FAILED"),
            Entry::Reply { text: "Counting in Redis now; the window test is next.".into() },
        ];
        let state = State { estimate: Some("wiring the counter".into()), waiting: vec!["Which store for counters?".into()] };
        let text = packet("backend", &backend_move(), Why::Drain, Some(&part), &entries, &state);
        for wanted in [
            "You are backend",
            "on Codex",
            "on Claude",
            "Claude's plan is running low",
            "nothing was lost",
            "T-42 Rate limit the login",
            "Acceptance criteria",
            "docs/plans/login.md",
            "Which store for counters?",
            "equipe/backend-t42",
            "/w/backend-t42",
            "0123456789",
            "src/login.rs | 12",
            "+let tries = 5;",
            "Uncommitted when it stopped",
            "estimate: counting in Redis",
            "In progress, as you last said: wiring the counter",
            "Next: where your last reply left it",
            "> Use Redis",
            "`Bash cargo test` → failed, exit 1",
            "login::five ... FAILED",
            "Person: Use Redis",
            "You: Counting in Redis",
            "## Before you change anything",
            "`git status`",
            "run the ticket's tests",
            "Say in one line what you will do next",
            "don't redo what is done",
            "AGENTS.md",
        ] {
            assert!(text.contains(wanted), "{wanted} in:\n{text}");
        }
        assert!(text.find("Person: Use Redis") < text.find("You: Counting"), "oldest first");
        assert!(!text.contains("> Earlier, at the desk"), "the person's words on this ticket, not before it");
        // At the desk: the same packet, none of the ticket's parts.
        let desk = packet("qa", &Move { member: "qa".into(), role: "qa".into(), ..backend_move() }, Why::Return, None, &entries[..1], &State::default());
        assert!(desk.contains("Codex's plan has room again") && desk.contains("carry on the conversation where it was") && desk.contains("> Earlier, at the desk"));
        assert!(!desk.contains("## The ticket") && !desk.contains("`git status`"));
    }

    #[test]
    fn a_packet_keeps_to_its_size_with_the_newest_of_a_long_conversation_verbatim() {
        let entries: Vec<Entry> = (0..200).map(|i| Entry::Reply { text: format!("step {i} {}", "x".repeat(400)) }).collect();
        let talk = brief_talk(&entries);
        assert!(talk.contains("step 199") && !talk.contains("step 0 ") && !talk.contains("step 179 "), "the last 20 messages");
        assert!(talk.contains("step 180 "));
        let huge: Vec<Entry> = (0..30).map(|i| Entry::Reply { text: format!("long {i} {}", "y".repeat(9_000)) }).collect();
        assert!(brief_talk(&huge).len() <= BRIEF_TALK + BRIEF_LINE, "{}", brief_talk(&huge).len());
        let big = Tree { diff: None, stat: " a.rs | 900 ++++".into(), ..Tree::default() };
        let part = brief_ticket(&ticket(&"y".repeat(20_000)), &big, &[]);
        assert!(part.contains("too long to put here"));
        let text = packet("backend", &backend_move(), Why::Drain, Some(&part), &huge, &State::default());
        // Bounded: about 10k tokens at most.
        assert!(text.len() < 45_000, "{}", text.len());
        // Kept for the screens whole, as sent; anything past the most, cut on a character.
        assert_eq!(packet_kept(&text), text);
        let wide = "é".repeat(equipe_protocol::engines::PACKET_MOST);
        let kept = packet_kept(&wide);
        assert!(kept.len() <= equipe_protocol::engines::PACKET_MOST && kept.len() >= equipe_protocol::engines::PACKET_MOST - 1 && kept.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_turn_that_keeps_answering_tells_its_plan_again_as_still_holding() {
        let start = std::time::Instant::now();
        let at = |mins: u64| start + Duration::from_secs(mins * 60);
        let plan = Activity::Plan { five_hour: None, seven_day: Some((0.33, 1_794_873_600)) };
        let mut told = ToldPlan::default();
        assert_eq!(told.heard(&Activity::Output(10), at(0)), None, "nothing told yet: nothing to hold");
        assert_eq!(told.heard(&plan, at(0)), None);
        assert_eq!(told.heard(&Activity::Output(20), at(4)), None, "said just now");
        assert_eq!(told.heard(&Activity::Text("hi".into()), at(6)), None, "only a request's answer says it holds");
        assert_eq!(told.heard(&Activity::Output(30), at(6)), Some(plan.clone()));
        assert_eq!(told.heard(&Activity::Output(40), at(8)), None, "and then not again at once");
        assert_eq!(told.heard(&Activity::Output(50), at(11)), Some(plan));
    }

    #[test]
    fn a_move_reaches_its_member_between_tickets_or_at_its_turns_end_when_now() {
        assert!(boundary(true, true, false), "a model or effort: at any turn's end");
        assert!(boundary(false, false, false), "off a ticket: as soon as it's free");
        assert!(!boundary(false, true, false), "on a ticket, gently: before its next one");
        assert!(boundary(false, true, true), "now: at the end of this turn, its ticket handed over");
    }
}
