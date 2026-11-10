//! Deciding in the person's own words, as the daemon does it — read with the
//! daemon's own rules (`equipe_core::by_words`) against the fake board: low
//! risk after its undo window, the rest a card, or (all by words) read back
//! and confirmed by the next message, outward ones counted down. The log of
//! where words went wrong is a made-up week, every flag in it.
//!
//! `FAKE_WORDS=read_back|countdown|undo|which|quote` has one waiting when a
//! screen attaches (merging T-42, pushing it, accepting T-12, a choice, or
//! paying in the person's name with the PM's words quoted) — and all by
//! words on; `FAKE_WORDS_LANG=en` reads back in English (Korean otherwise).

use std::sync::Mutex;

use equipe_core::by_words::{self as rules, Ask, Step, Waiting};
use equipe_core::i18n::Lang;
use equipe_core::protocol::by_words::{Entry, Flag, Heard, Kind, Live, Outcome, Quote, Reply, Stage, Tier, Tongues, Via};
use equipe_core::protocol::{Command, Event, Ticket};
use tokio::sync::broadcast;

const SQUAD: &str = "demo";
const PM: u64 = 1;

fn lang() -> Lang {
    std::env::var("FAKE_WORDS_LANG").ok().and_then(|l| Lang::parse(&l)).unwrap_or(Lang::Ko)
}

/// "All by words" for the squad, as the daemon keeps it: on with `FAKE_WORDS`.
static ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn all() -> bool {
    ALL.load(std::sync::atomic::Ordering::Relaxed) || std::env::var_os("FAKE_WORDS").is_some()
}

pub fn set_all(on: bool, events: &broadcast::Sender<Event>) {
    ALL.store(on, std::sync::atomic::Ordering::Relaxed);
    let _ = events.send(Event::WordsAllChanged { squad: SQUAD.into(), on });
}

/// The languages read: the fake's, and English.
fn tongues() -> Tongues {
    Tongues::of(&[lang().code(), "ko"])
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

struct Talk {
    live: Live,
    ask: Option<Ask>,
    choices: Vec<Ask>,
}

fn talks() -> &'static Mutex<Vec<Talk>> {
    static TALKS: std::sync::OnceLock<Mutex<Vec<Talk>>> = std::sync::OnceLock::new();
    TALKS.get_or_init(|| Mutex::new(Vec::new()))
}

#[allow(clippy::too_many_arguments)]
fn live(id: u64, stage: Stage, tier: Tier, kind: Kind, read_back: String, items: Vec<String>, said: &str, wait: u64) -> Live {
    Live { id, session: PM, stage, tier, kind, read_back, items, quotes: Vec::new(), until: now_ms() + wait, said: said.into(), via: Via::Typed, asked_again: 0, device: None }
}

/// What waits when a screen attaches (`FAKE_WORDS`), read against the fake board.
pub fn on_hello(tickets: &[Ticket]) -> Vec<Live> {
    let mut all = talks().lock().unwrap();
    if all.is_empty() && let Some(mode) = std::env::var("FAKE_WORDS").ok().filter(|m| !m.is_empty()) {
        let w = Waiting { tickets, ..Default::default() };
        let (ask, stage, wait, said) = match mode.as_str() {
            "countdown" => (Ask::Tickets { verb: rules::Verb::Push, tickets: vec![42] }, Stage::Countdown, 10_000, "T-42 merge push"),
            "undo" => (Ask::Tickets { verb: rules::Verb::Accept, tickets: vec![12] }, Stage::Undo, 5_000, "accept T-12"),
            "which" => (Ask::Tickets { verb: rules::Verb::Merge, tickets: vec![42] }, Stage::Which, 120_000, "approve"),
            _ => (Ask::Tickets { verb: rules::Verb::Merge, tickets: vec![42] }, Stage::ReadBack, 120_000, "merge T-42"),
        };
        let back = rules::read_back(lang(), &ask, &w);
        let kind = rules::kind(&ask, &w);
        let mut shown = live(9_100, stage, kind.tier(), kind, back.head, back.items, said, wait);
        if mode == "quote" {
            // Paying in the person's name: equipe's line, and the PM's own words quoted as the PM's.
            shown.kind = Kind::ActInName;
            shown.tier = Tier::Outward;
            shown.read_back = equipe_core::i18n::core(lang()).fill("words.do.act_in_name", &[("member", "pm".into())]);
            shown.items = vec![equipe_core::i18n::core(lang()).fill("words.item.on", &[("ticket", "T-117".into())])];
            shown.quotes = vec![Quote::of("pm", "Pay $99.00 a year to Apple for the developer account"), Quote::of("pm", "App Store submission needs it")];
            shown.said = "approve".into();
        }
        let mut choices = Vec::new();
        if stage == Stage::Which {
            choices = vec![ask.clone(), Ask::Tickets { verb: rules::Verb::Accept, tickets: vec![71] }];
            shown.read_back = equipe_core::i18n::core(lang()).get("words.which").to_owned();
            shown.items = rules::which_items(lang(), &choices, &w);
        }
        all.push(Talk { live: shown, ask: (stage != Stage::Which).then_some(ask), choices });
    }
    all.iter().map(|t| t.live.clone()).collect()
}


fn close(id: u64, outcome: Outcome, events: &broadcast::Sender<Event>) -> Option<Ask> {
    let talk = {
        let mut all = talks().lock().unwrap();
        let at = all.iter().position(|t| t.live.id == id)?;
        all.remove(at)
    };
    let key = match outcome {
        Outcome::Done => "words.done",
        Outcome::Undone => "words.undone",
        Outcome::Cancelled => "words.cancelled",
        Outcome::Lapsed => "words.lapsed",
        _ => "words.held",
    };
    let text = equipe_core::i18n::core(lang()).fill(key, &[("what", talk.live.read_back.clone()), ("said", talk.live.said.clone())]);
    let _ = events.send(Event::WordsClosed { squad: SQUAD.into(), id, outcome, text });
    talk.ask
}

/// In `ms`, unless it moved on: done (the board's commands, run on the fake board), or lapsed.
fn due(id: u64, ms: u64, until: u64, events: broadcast::Sender<Event>) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        let stage = talks().lock().unwrap().iter().find(|t| t.live.id == id && t.live.until == until).map(|t| t.live.stage);
        match stage {
            Some(Stage::Undo | Stage::Countdown) => {
                if let Some(ask) = close(id, Outcome::Done, &events) {
                    run(&ask, &events);
                }
            }
            Some(_) => {
                close(id, Outcome::Lapsed, &events);
            }
            None => {}
        }
    });
}

/// What a decision sends — its card's buttons' commands — run on the fake board.
fn run(ask: &Ask, events: &broadcast::Sender<Event>) {
    let tickets = super::tickets().lock().unwrap().clone();
    let w = Waiting { tickets: &tickets, ..Default::default() };
    let said = equipe_core::protocol::by_words::Said { via: Via::Typed, utterance: String::new() };
    super::run_commands(rules::commands(SQUAD, ask, &w, &said), events);
}

/// The person's message, as the daemon reads it. Whether it goes on to the member.
pub fn on_say(text: &str, pasted: bool, events: &broadcast::Sender<Event>) -> bool {
    if pasted {
        return true;
    }
    let tickets = super::tickets().lock().unwrap().clone();
    let tickets = tickets.as_slice();
    let lang = lang();
    let t = tongues();
    let answering = talks().lock().unwrap().iter().filter(|t| matches!(t.live.stage, Stage::ReadBack | Stage::Which)).map(|t| (t.live.id, t.live.stage, t.live.tier, t.choices.len(), t.live.asked_again)).next_back();
    let waiting_stop = talks().lock().unwrap().iter().filter(|t| matches!(t.live.stage, Stage::Undo | Stage::Countdown)).map(|t| (t.live.id, t.live.stage, t.live.tier)).next_back();
    let pick = answering.or(waiting_stop.filter(|_| equipe_core::protocol::by_words::stops(&t, text)).map(|(id, stage, tier)| (id, stage, tier, 0, 0)));
    if let Some((id, stage, tier, count, again)) = pick {
        let (step, _) = rules::answer(&t, stage, tier, false, text, count, again);
        match step {
            Step::Pass => return true,
            Step::Run => {
                if let Some(ask) = close(id, Outcome::Done, events) {
                    run(&ask, events);
                }
            }
            Step::Countdown => {
                let until = now_ms() + 10_000;
                let shown = {
                    let mut all = talks().lock().unwrap();
                    let Some(t) = all.iter_mut().find(|t| t.live.id == id) else { return false };
                    t.live.stage = Stage::Countdown;
                    t.live.until = until;
                    t.live.clone()
                };
                let _ = events.send(Event::WordsLive { squad: SQUAD.into(), live: shown });
                due(id, 10_000, until, events.clone());
            }
            Step::Again => {
                let shown = {
                    let mut all = talks().lock().unwrap();
                    let Some(t) = all.iter_mut().find(|t| t.live.id == id) else { return false };
                    t.live.asked_again += 1;
                    t.live.clone()
                };
                let _ = events.send(Event::WordsLive { squad: SQUAD.into(), live: shown });
            }
            Step::Choose(n) => {
                let w = Waiting { tickets, ..Default::default() };
                let shown = {
                    let mut all = talks().lock().unwrap();
                    let Some(t) = all.iter_mut().find(|t| t.live.id == id) else { return false };
                    let Some(ask) = t.choices.get(n - 1).cloned() else { return false };
                    let back = rules::read_back(lang, &ask, &w);
                    let kind = rules::kind(&ask, &w);
                    t.live = Live { stage: Stage::ReadBack, tier: kind.tier(), kind, read_back: back.head, items: back.items, quotes: back.quotes, ..t.live.clone() };
                    t.ask = Some(ask);
                    t.choices.clear();
                    t.live.clone()
                };
                let _ = events.send(Event::WordsLive { squad: SQUAD.into(), live: shown });
            }
            Step::Close { outcome, pass } => {
                close(id, outcome, events);
                return pass;
            }
        }
        return false;
    }
    let w = Waiting { tickets, ..Default::default() };
    let asks = rules::read(text, &w, &t);
    let [ask] = asks.as_slice() else { return true };
    let kind = rules::kind(ask, &w);
    let back = rules::read_back(lang, ask, &w);
    let (head, items) = (back.head, back.items);
    let id = 9_200 + talks().lock().unwrap().len() as u64;
    match kind.tier() {
        Tier::Low => {
            let shown = live(id, Stage::Undo, Tier::Low, kind, head, items, text, 5_000);
            let until = shown.until;
            talks().lock().unwrap().push(Talk { live: shown.clone(), ask: Some(ask.clone()), choices: Vec::new() });
            let _ = events.send(Event::WordsLive { squad: SQUAD.into(), live: shown });
            due(id, 5_000, until, events.clone());
        }
        tier if all() => {
            let mut shown = live(id, Stage::ReadBack, tier, kind, head, items, text, 120_000);
            shown.quotes = back.quotes;
            talks().lock().unwrap().push(Talk { live: shown.clone(), ask: Some(ask.clone()), choices: Vec::new() });
            let _ = events.send(Event::WordsLive { squad: SQUAD.into(), live: shown });
        }
        _ => {
            if let Some((verdict, tickets)) = rules::card_for(&asks) {
                super::run_commands(vec![Command::ProposeAccept { squad: SQUAD.into(), session: PM, verdict, tickets, note: String::new() }], events);
            }
        }
    }
    true
}

/// The Undo (or Cancel, or Leave) button.
pub fn on_undo(id: u64, events: &broadcast::Sender<Event>) {
    let stage = talks().lock().unwrap().iter().find(|t| t.live.id == id).map(|t| t.live.stage);
    let outcome = match stage {
        Some(Stage::Undo) => Outcome::Undone,
        Some(Stage::Countdown) => Outcome::Cancelled,
        Some(_) => Outcome::Held,
        None => return,
    };
    close(id, outcome, events);
}

/// A made-up week of decisions in words: every outcome and flag.
pub fn log() -> Vec<Entry> {
    let day = now_ms() - 3 * 60 * 60 * 1000;
    let heard = |words: &str, read: Reply, at: u64, via: Via| Heard { words: words.into(), read, at, via };
    let entry = |id: u64, at: u64, kind: Kind, via: Via, said: &str, read_back: &str, answers: Vec<Heard>, outcome: Outcome, flags: Vec<Flag>| Entry {
        id,
        session: PM,
        kind,
        tier: kind.tier(),
        via,
        all_by_words: kind.tier() != Tier::Low,
        said: said.into(),
        read_back: read_back.into(),
        items: vec![],
        quotes: if kind == Kind::AllowOutward { vec![Quote::of("frontend", "to ship the fix tonight")] } else { vec![] },
        asked_again: answers.iter().filter(|a| a.read == Reply::Unclear).count() as u32,
        decided_at: answers.last().map(|a| a.at).or(Some(at + 5_000)),
        answers,
        tickets: vec![42],
        said_at: at,
        read_back_at: (!read_back.is_empty()).then_some(at),
        closed_at: at + 15_000,
        outcome,
        chose: false,
        device: None,
        flags,
    };
    vec![
        entry(6, day + 50 * 60_000, Kind::Push, Via::Voice, "merge T-42 and push", "Merge T-42 into main and push main to origin", vec![heard("yes", Reply::Yes, day + 50 * 60_000 + 4_000, Via::Voice), heard("wait, cancel", Reply::Cancel, day + 50 * 60_000 + 9_000, Via::Voice)], Outcome::Cancelled, vec![Flag::Cancelled]),
        entry(5, day + 40 * 60_000, Kind::Merge, Via::Typed, "merge T-42", "Merge T-42 into main, on this Mac", vec![heard("hmm", Reply::Unclear, day + 40 * 60_000 + 6_000, Via::Typed), heard("ok", Reply::Yes, day + 40 * 60_000 + 14_000, Via::Typed)], Outcome::Done, vec![Flag::AskedAgain, Flag::Ambiguous, Flag::RevertedLater]),
        entry(4, day + 30 * 60_000, Kind::AcceptTask, Via::Dictated, "accept T-12", "", vec![heard("undo", Reply::Undo, day + 30 * 60_000 + 2_000, Via::Typed)], Outcome::Undone, vec![Flag::Undone]),
        entry(3, day + 20 * 60_000, Kind::AllowOutward, Via::Typed, "allow it", "Let frontend reach outside the squad, once: git push origin main", vec![heard("not yet", Reply::No, day + 20 * 60_000 + 3_000, Via::Typed)], Outcome::Held, vec![Flag::Held]),
        entry(2, day + 10 * 60_000, Kind::KeepIdea, Via::Typed, "keep T-5 as an idea", "", vec![], Outcome::Done, vec![]),
        entry(1, day, Kind::RunRoutine, Via::Voice, "run the weekly report", "", vec![], Outcome::Done, vec![]),
    ]
}
