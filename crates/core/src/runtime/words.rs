//! Deciding in the person's own words (`crate::by_words`): a screen's
//! [`Command::Say`] — the person's own message — read before it goes on to
//! the member, in the person's languages. Low-risk requests it names run
//! after a short undo window; in "all by words" (the squad's setting, kept
//! here and changed only from the Mac's screens), the rest are read back and
//! wait for the person's next own message to be a clear yes — outward ones
//! then count down a cancel window; otherwise they keep their card. What
//! waits is shown on its card over the composer, never in a member's reply;
//! a stopped daemon logs what was still waiting as dropped. Every decision
//! made in words goes into the squad's log, kept beside its journal as long
//! as the journal keeps its entries, for the person to find where words went
//! wrong.
//!
//! What runs is what the request's card would send, through
//! [`Runtime::handle`] — the person's own commands. Nothing a member says
//! reaches here: only `Say`, from a screen.

use equipe_protocol::by_words::{self as rules, Entry, Heard, Kind, Live, Outcome, Reply, Said, Stage, Tier, Tongues, Via};

use super::*;
use crate::by_words::{self, Ask, Step, Waiting};

/// A decision in words waiting on the person.
pub(super) struct Talk {
    squad: String,
    live: Live,
    /// What it does — `None` while the person chooses among `choices`.
    ask: Option<Ask>,
    choices: Vec<Ask>,
    entry: Entry,
}

/// The words requests waiting, by id; and whether one runs now (its
/// commands are the words', already kept in the log).
#[derive(Default)]
pub(super) struct Words {
    talks: HashMap<u64, Talk>,
    running: bool,
}

/// What waits on the person, owned for a moment to read the words against.
struct Owned {
    tickets: Vec<Ticket>,
    approvals: Vec<Approval>,
    accepts: Vec<equipe_protocol::accept::AcceptProposal>,
    routines: Vec<Routine>,
    routine_proposals: Vec<equipe_protocol::routines::RoutineProposal>,
    team_changes: Vec<equipe_protocol::team::TeamChange>,
    engine_proposals: Vec<equipe_protocol::engines::Proposal>,
}

impl Owned {
    fn view(&self) -> Waiting<'_> {
        Waiting {
            tickets: &self.tickets,
            approvals: &self.approvals,
            accepts: &self.accepts,
            routines: &self.routines,
            routine_proposals: &self.routine_proposals,
            team_changes: &self.team_changes,
            engine_proposals: &self.engine_proposals,
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The most of the log read back for the screen.
const LOG_SHOWN: usize = 500;

/// The language codes a detected language's name stands for.
fn codes_of(name: &str) -> &'static [&'static str] {
    match name {
        "Korean" => &["ko"],
        "Japanese" => &["ja"],
        "Chinese" => &["zh-Hans", "zh-Hant"],
        "German" => &["de"],
        "French" => &["fr"],
        "Spanish" => &["es"],
        _ => &[],
    }
}

/// The language a letter's script is written in, where only one is.
fn script_of(c: char) -> Option<&'static str> {
    match c {
        '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' => Some("ko"),
        '\u{3040}'..='\u{30FF}' => Some("ja"),
        _ => None,
    }
}

impl Runtime {
    fn waiting(&mut self, squad: &str) -> Owned {
        let mut approvals: Vec<Approval> = self.pending.values().filter(|p| p.squad == squad).map(|p| p.approval.clone()).collect();
        approvals.sort_by_key(|a| a.id);
        Owned {
            tickets: self.tickets(squad),
            approvals,
            accepts: self.accepts(squad).clone(),
            routines: self.routines(squad),
            routine_proposals: self.routine_proposals(squad).clone(),
            team_changes: self.team_changes(squad).clone(),
            engine_proposals: self.engines.proposals(squad),
        }
    }

    /// The languages the person's words are read in: the screen's, the one
    /// they write `text` in when it shows, and English.
    fn tongues(&self, text: &str) -> Tongues {
        let mut codes = vec![self.config.lang.code()];
        if let Some(name) = crate::language::detect(text) {
            codes.extend_from_slice(codes_of(name));
        }
        // A reply too short to detect still shows its script: one syllable, or letters alone.
        codes.extend(text.chars().find_map(script_of));
        Tongues::of(&codes)
    }

    /// The squad's words requests waiting, for a screen that attaches.
    pub(super) fn words_live(&self, squad: &str) -> Vec<Live> {
        let mut live: Vec<Live> = self.words.talks.values().filter(|t| t.squad == squad).map(|t| t.live.clone()).collect();
        live.sort_by_key(|l| l.id);
        live
    }

    /// "All by words" for the squad, as the person set it at the Mac.
    pub(super) fn set_words_all(&mut self, squad: String, on: bool) {
        let kept = self.journal(&squad).map(|j| j.set_words_all(on));
        match kept {
            Some(Ok(())) => self.publish(Event::WordsAllChanged { squad, on }),
            Some(Err(why)) => crate::daemon_log!("equipe: {squad}'s all by words wasn't kept: {why}"),
            None => {}
        }
    }

    /// The person's own message: an answer to what waits on them in words,
    /// or a new request in words — and, unless it was only an answer, on to
    /// the member as they said it. What the screen says of "all by words" is
    /// not asked: the squad's own setting is.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn say(&mut self, squad: String, session: SessionId, text: String, via: Via, _all: bool, later: bool, pasted: bool, about: Option<Option<String>>, device: Option<String>) {
        let ours = self.sessions.get(&session).is_some_and(|s| s.squad.as_deref() == Some(squad.as_str()));
        let passes = if ours && !pasted { self.words_heard(&squad, session, &text, via, device.as_deref()).await } else { true };
        if passes {
            let command = if later { Command::SubmitLater { session, text, about } } else { Command::Submit { session, text, about } };
            Box::pin(self.handle(command)).await;
        }
    }

    /// Read the person's words for a decision. Whether they go on to the member.
    async fn words_heard(&mut self, squad: &str, session: SessionId, text: &str, via: Via, device: Option<&str>) -> bool {
        let tongues = self.tongues(text);
        let all = self.journal(squad).is_some_and(|j| j.words_all());
        // An answer to what was read back, or put to them to choose: theirs, from the same screen.
        let mine = |t: &Talk| t.squad == squad && t.live.device.as_deref() == device;
        if let Some(id) = self.words.talks.values().filter(|t| mine(t) && matches!(t.live.stage, Stage::ReadBack | Stage::Which)).map(|t| t.live.id).max() {
            return self.words_answered(id, text, via, &tongues).await;
        }
        // "Undo", "cancel" while one counts down: the latest stops.
        if rules::stops(&tongues, text)
            && let Some(id) = self.words.talks.values().filter(|t| mine(t) && matches!(t.live.stage, Stage::Undo | Stage::Countdown)).map(|t| t.live.id).max()
        {
            return self.words_answered(id, text, via, &tongues).await;
        }
        // A new request.
        let owned = self.waiting(squad);
        let asks = by_words::read(text, &owned.view(), &tongues);
        if asks.is_empty() {
            return true;
        }
        let lang = self.config.lang;
        let said = text.trim().to_owned();
        let device = device.map(str::to_owned);
        let entry = |id: u64, kind: Kind, tickets: Vec<TicketId>| Entry {
            id,
            session,
            kind,
            tier: kind.tier(),
            via,
            all_by_words: all,
            said: said.clone(),
            read_back: String::new(),
            items: Vec::new(),
            quotes: Vec::new(),
            answers: Vec::new(),
            tickets,
            said_at: now_ms(),
            read_back_at: None,
            decided_at: None,
            closed_at: 0,
            outcome: Outcome::Done,
            asked_again: 0,
            chose: false,
            device: device.clone(),
            flags: Vec::new(),
        };
        let id = equipe_protocol::new_session_id();
        if let [ask] = asks.as_slice() {
            let kind = by_words::kind(ask, &owned.view());
            let tier = kind.tier();
            if kind.reach() != rules::Reach::Words {
                return true;
            }
            let back = by_words::read_back(lang, ask, &owned.view());
            let mut entry = entry(id, kind, by_words::tickets_of(ask, &owned.view()));
            entry.read_back = back.head.clone();
            entry.items = back.items.clone();
            entry.quotes = back.quotes.clone();
            let live = |stage: Stage, until: u64| Live { id, session, stage, tier, kind, read_back: back.head.clone(), items: back.items.clone(), quotes: back.quotes.clone(), until, said: said.clone(), via, asked_again: 0, device: device.clone() };
            match tier {
                Tier::Low => {
                    entry.read_back.clear();
                    let until = now_ms() + rules::UNDO_MS;
                    self.words_start(Talk { squad: squad.to_owned(), live: live(Stage::Undo, until), ask: Some(ask.clone()), choices: Vec::new(), entry }, rules::UNDO_MS);
                }
                // Read back on its card over the composer: never a line inside a member's reply.
                _ if all => {
                    entry.read_back_at = Some(now_ms());
                    let until = now_ms() + rules::READ_BACK_MS;
                    self.words_start(Talk { squad: squad.to_owned(), live: live(Stage::ReadBack, until), ask: Some(ask.clone()), choices: Vec::new(), entry }, rules::READ_BACK_MS);
                }
                // Cards decide it: accepting or merging in words raises one, as it always did.
                _ => {
                    if let Some((verdict, tickets)) = by_words::card_for(&asks) {
                        self.propose_accept_from_words(squad, session, verdict, &tickets, "");
                        entry.outcome = Outcome::NeedsCard;
                        entry.closed_at = now_ms();
                        self.words_keep(squad, &entry);
                    }
                }
            }
            return true;
        }
        // Several fit: the person picks one by its number — in "all by words", or when each is low.
        let view = owned.view();
        let every_low = asks.iter().all(|a| by_words::tier(a, &view) == Tier::Low);
        if !(all || every_low) {
            return true;
        }
        let items = by_words::which_items(lang, &asks, &view);
        let head = self.said("words.which", &[]);
        let tier = asks.iter().map(|a| by_words::tier(a, &view)).max().unwrap_or(Tier::High);
        let kind = by_words::kind(&asks[0], &view);
        let mut entry = entry(id, kind, Vec::new());
        entry.read_back = head.clone();
        entry.items = items.clone();
        entry.read_back_at = Some(now_ms());
        let until = now_ms() + rules::READ_BACK_MS;
        let live = Live { id, session, stage: Stage::Which, tier, kind, read_back: head, items, quotes: Vec::new(), until, said, via, asked_again: 0, device };
        self.words_start(Talk { squad: squad.to_owned(), live, ask: None, choices: asks, entry }, rules::READ_BACK_MS);
        true
    }

    /// Keep it waiting, show it, and time it.
    fn words_start(&mut self, talk: Talk, wait_ms: u64) {
        let (id, squad, live) = (talk.live.id, talk.squad.clone(), talk.live.clone());
        self.words.talks.insert(id, talk);
        self.publish(Event::WordsLive { squad: squad.clone(), live });
        self.words_save_live(&squad);
        self.words_timer(id, wait_ms);
    }

    fn words_timer(&self, id: u64, wait_ms: u64) {
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            let _ = internal.send(Internal::WordsDue(id));
        });
    }

    /// Shown anew on every screen, and kept against a restart.
    fn words_changed(&mut self, id: u64) {
        let Some(talk) = self.words.talks.get(&id) else { return };
        let (squad, live) = (talk.squad.clone(), talk.live.clone());
        self.publish(Event::WordsLive { squad: squad.clone(), live });
        self.words_save_live(&squad);
    }

    /// The person's message, answering talk `id`. Whether it goes on to the member.
    async fn words_answered(&mut self, id: u64, text: &str, via: Via, tongues: &Tongues) -> bool {
        let Some(talk) = self.words.talks.get(&id) else { return true };
        let (stage, tier, count, again) = (talk.live.stage, talk.live.tier, talk.choices.len(), talk.live.asked_again);
        // Said aloud or dictated: a yes to what is high risk is short and on its own.
        let strict = via != Via::Typed;
        let (step, read) = by_words::answer(tongues, stage, tier, strict, text, count, again);
        let answering = matches!(stage, Stage::ReadBack | Stage::Which);
        if (answering || read != Reply::Other)
            && let Some(talk) = self.words.talks.get_mut(&id)
        {
            talk.entry.answers.push(Heard { words: text.trim().to_owned(), read, at: now_ms(), via });
        }
        match step {
            Step::Pass => true,
            Step::Run => {
                self.words_decided(id);
                self.words_run(id).await;
                false
            }
            Step::Countdown => {
                self.words_decided(id);
                let Some(talk) = self.words.talks.get_mut(&id) else { return false };
                talk.live.stage = Stage::Countdown;
                talk.live.until = now_ms() + rules::CANCEL_MS;
                self.words_changed(id);
                self.words_timer(id, rules::CANCEL_MS);
                false
            }
            Step::Choose(n) => {
                let Some(talk) = self.words.talks.get_mut(&id) else { return false };
                let Some(ask) = talk.choices.get(n - 1).cloned() else { return false };
                let squad = talk.squad.clone();
                let owned = self.waiting(&squad);
                let view = owned.view();
                let kind = by_words::kind(&ask, &view);
                let tier = kind.tier();
                if kind.reach() != rules::Reach::Words {
                    self.words_close(id, Outcome::Held);
                    return false;
                }
                let back = by_words::read_back(self.config.lang, &ask, &view);
                let tickets = by_words::tickets_of(&ask, &view);
                let Some(talk) = self.words.talks.get_mut(&id) else { return false };
                talk.ask = Some(ask);
                talk.choices.clear();
                talk.entry.chose = true;
                talk.entry.kind = kind;
                talk.entry.tier = tier;
                talk.entry.tickets = tickets;
                talk.entry.read_back = back.head.clone();
                talk.entry.items = back.items.clone();
                talk.entry.quotes = back.quotes.clone();
                talk.live.kind = kind;
                talk.live.tier = tier;
                talk.live.read_back = back.head;
                talk.live.items = back.items;
                talk.live.quotes = back.quotes;
                talk.live.asked_again = 0;
                // A low one, picked: as plainly said; anything else, read back.
                let (stage, wait) = if tier == Tier::Low { (Stage::Undo, rules::UNDO_MS) } else { (Stage::ReadBack, rules::READ_BACK_MS) };
                talk.live.stage = stage;
                talk.live.until = now_ms() + wait;
                self.words_changed(id);
                self.words_timer(id, wait);
                false
            }
            // Asked again on its card, not in the conversation.
            Step::Again => {
                let Some(talk) = self.words.talks.get_mut(&id) else { return false };
                talk.live.asked_again += 1;
                talk.entry.asked_again += 1;
                talk.live.until = now_ms() + rules::READ_BACK_MS;
                self.words_changed(id);
                self.words_timer(id, rules::READ_BACK_MS);
                false
            }
            Step::Close { outcome, pass } => {
                self.words_decided(id);
                self.words_close(id, outcome);
                pass
            }
        }
    }

    fn words_decided(&mut self, id: u64) {
        if let Some(talk) = self.words.talks.get_mut(&id) {
            talk.entry.decided_at.get_or_insert(now_ms());
        }
    }

    /// Its time is up: what was waiting for the undo or cancel window runs;
    /// a read-back nobody answered lapses to its card.
    pub(super) async fn words_due(&mut self, id: u64) {
        let Some(talk) = self.words.talks.get(&id) else { return };
        // A timer from before a step on (asked again, chosen): the newer one counts.
        if talk.live.until > now_ms() + 50 {
            return;
        }
        match talk.live.stage {
            Stage::Undo | Stage::Countdown => self.words_run(id).await,
            Stage::ReadBack | Stage::Which => self.words_close(id, Outcome::Lapsed),
        }
    }

    /// The Undo (or Cancel, or Leave) button.
    pub(super) fn undo_words(&mut self, squad: &str, id: u64) {
        let Some(talk) = self.words.talks.get(&id).filter(|t| t.squad == squad) else { return };
        let outcome = match talk.live.stage {
            Stage::Undo => Outcome::Undone,
            Stage::Countdown => Outcome::Cancelled,
            Stage::ReadBack | Stage::Which => Outcome::Held,
        };
        self.words_decided(id);
        self.words_close(id, outcome);
    }

    /// Do it: what its card would send, unless it no longer waits as it did.
    async fn words_run(&mut self, id: u64) {
        let Some(talk) = self.words.talks.get(&id) else { return };
        let squad = talk.squad.clone();
        let Some(ask) = talk.ask.clone() else { return self.words_close(id, Outcome::Lapsed) };
        let said = Said { via: talk.live.via, utterance: talk.entry.answers.iter().rev().find(|a| a.read == Reply::Yes).map_or_else(|| talk.live.said.clone(), |a| a.words.clone()) };
        let owned = self.waiting(&squad);
        if !by_words::still_waits(&ask, &owned.view()) {
            return self.words_close(id, Outcome::Moot);
        }
        let commands = by_words::commands(&squad, &ask, &owned.view(), &said);
        self.words.running = true;
        for command in commands {
            Box::pin(self.handle(command)).await;
        }
        self.words.running = false;
        self.words_close(id, Outcome::Done);
    }

    /// It ends: every screen told — its card goes, its line shown under the
    /// composer — and it is logged. The conversation gets the one line only
    /// between replies: never inside one a member is writing.
    fn words_close(&mut self, id: u64, outcome: Outcome) {
        let Some(mut talk) = self.words.talks.remove(&id) else { return };
        let what = talk.live.read_back.clone();
        let key = match outcome {
            Outcome::Done => "words.done",
            Outcome::Undone => "words.undone",
            Outcome::Cancelled => "words.cancelled",
            Outcome::Held => "words.held",
            Outcome::Lapsed => "words.lapsed",
            Outcome::Moot | Outcome::NeedsCard | Outcome::Dropped => "words.moot",
        };
        // Choosing, nothing was picked: there is no one thing to name.
        let what = if talk.ask.is_none() { talk.live.said.clone() } else { what };
        let said = talk.entry.answers.iter().rev().find(|a| a.read == Reply::Yes).map_or_else(|| talk.live.said.clone(), |a| a.words.clone());
        let text = self.said(key, &[("what", what), ("said", said)]);
        self.publish(Event::WordsClosed { squad: talk.squad.clone(), id, outcome, text: text.clone() });
        let session = talk.live.session;
        if !self.sessions.get(&session).is_some_and(Session::is_working) {
            self.publish(Event::Notice { session, text });
        }
        talk.entry.outcome = outcome;
        talk.entry.closed_at = now_ms();
        self.words_keep(&talk.squad, &talk.entry);
        self.words_save_live(&talk.squad);
    }

    /// What waits in `squad` now, kept: should the daemon stop, its next
    /// start logs it as dropped.
    fn words_save_live(&self, squad: &str) {
        let path = by_words::live_path(&self.config.home, squad);
        let lines: Vec<String> = self.words.talks.values().filter(|t| t.squad == squad).filter_map(|t| serde_json::to_string(&t.entry).ok()).collect();
        let kept = if lines.is_empty() { std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) } else { std::fs::write(&path, lines.join("\n") + "\n") };
        if let Err(why) = kept {
            crate::daemon_log!("equipe: {squad}'s words waiting weren't kept: {why}");
        }
    }

    /// At start, for every squad: what waited in words when the daemon
    /// stopped, logged as dropped.
    pub(super) fn words_dropped_everywhere(&mut self) {
        let Ok(dirs) = std::fs::read_dir(self.config.home.join("squads")) else { return };
        let squads: Vec<String> = dirs.flatten().filter(|d| d.path().join("words-live.jsonl").is_file()).filter_map(|d| d.file_name().into_string().ok()).collect();
        for squad in squads {
            self.words_dropped_at_restart(&squad);
        }
    }

    /// What waited in `squad`'s words when the daemon stopped, logged as
    /// dropped — nothing of it happened, and its card is what is left.
    fn words_dropped_at_restart(&mut self, squad: &str) {
        let path = by_words::live_path(&self.config.home, squad);
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let _ = std::fs::remove_file(&path);
        for mut entry in by_words::parse_log(&text) {
            entry.outcome = Outcome::Dropped;
            entry.closed_at = now_ms();
            self.words_keep(squad, &entry);
        }
    }

    /// One more line in the squad's log of decisions in words — what is
    /// older than the journal keeps its entries let go.
    fn words_keep(&mut self, squad: &str, entry: &Entry) {
        let path = by_words::log_path(&self.config.home, squad);
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let mut entries = by_words::prune(by_words::parse_log(&before), now_ms(), self.config.retention_days);
        entries.push(entry.clone());
        let text: String = entries.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
        let kept = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, text)
        })();
        if let Err(why) = kept {
            crate::daemon_log!("equipe: {squad}'s words log wasn't kept: {why}");
        }
    }

    /// The log emptied, at the person's word on the Mac.
    pub(super) fn clear_words_log(&mut self, squad: String) {
        match std::fs::remove_file(by_words::log_path(&self.config.home, &squad)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return crate::daemon_log!("equipe: {squad}'s words log wasn't deleted: {why}"),
        }
        self.publish(Event::WordsLog { squad, entries: Vec::new() });
    }

    /// A decision a device made sure of in words itself (a phone's call
    /// mode): logged as said, with no read-back of ours. The words' own
    /// commands are logged where they were decided.
    pub(super) fn words_said_elsewhere(&mut self, squad: &str, kind: Kind, tickets: Vec<TicketId>, said: &Said, outcome: Outcome) {
        if self.words.running {
            return;
        }
        let at = now_ms();
        let entry = Entry {
            id: equipe_protocol::new_session_id(),
            session: 0,
            kind,
            tier: kind.tier(),
            via: said.via,
            all_by_words: false,
            said: said.utterance.clone(),
            read_back: String::new(),
            items: Vec::new(),
            quotes: Vec::new(),
            answers: Vec::new(),
            tickets,
            said_at: at,
            read_back_at: None,
            decided_at: Some(at),
            closed_at: at,
            outcome,
            asked_again: 0,
            chose: false,
            device: None,
            flags: Vec::new(),
        };
        self.words_keep(squad, &entry);
    }

    /// A phone's `decide` said in words: logged before it is answered.
    pub(super) fn words_decide_said(&mut self, squad: &str, approval: u64, decision: Decision, said: Option<&Said>) {
        let Some(said) = said else { return };
        let Some(pending) = self.pending.get(&approval).filter(|p| p.squad == squad) else { return };
        let kind = if decision == Decision::Always { Kind::AllowAlways } else { by_words::approval_kind(&pending.approval) };
        let tickets = pending.approval.ticket.into_iter().collect();
        let outcome = if decision == Decision::Deny { Outcome::Held } else { Outcome::Done };
        self.words_said_elsewhere(squad, kind, tickets, said, outcome);
    }

    /// The log, newest first, each flagged where it may have gone wrong.
    pub(super) fn words_log(&mut self, squad: String) {
        let text = std::fs::read_to_string(by_words::log_path(&self.config.home, &squad)).unwrap_or_default();
        let mut entries = by_words::prune(by_words::parse_log(&text), now_ms(), self.config.retention_days);
        let skip = entries.len().saturating_sub(LOG_SHOWN);
        entries.drain(..skip);
        let since = entries.iter().map(|e| e.said_at / 1000).min().unwrap_or(0);
        let events = self.journal(&squad).and_then(|j| j.ticket_events_since(since).ok()).unwrap_or_default();
        let mut entries = by_words::flagged(entries, &events);
        entries.reverse();
        self.publish(Event::WordsLog { squad, entries });
    }

    /// The week's line on decisions in words, for the retro's facts.
    pub(super) fn words_week(&mut self, squad: &str, from: u64) -> Option<String> {
        let text = std::fs::read_to_string(by_words::log_path(&self.config.home, squad)).ok()?;
        let entries = by_words::parse_log(&text);
        let events = self.journal(squad).and_then(|j| j.ticket_events_since(from).ok()).unwrap_or_default();
        by_words::week_line(&by_words::flagged(entries, &events), from * 1000)
    }

    /// Tickets a words request waiting on the person already does something
    /// to: the PM proposes no card for them meanwhile.
    pub(super) fn words_cover(&self, squad: &str, tickets: &[TicketId]) -> bool {
        !tickets.is_empty() && tickets.iter().all(|t| self.words.talks.values().any(|talk| talk.squad == squad && talk.entry.tickets.contains(t)))
    }
}
