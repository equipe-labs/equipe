//! The live line at the foot of a member's conversation: while it has a turn
//! under way — or summarizes its conversation, waits on the person's sign-in
//! or approval, or has the person's message waiting — one quiet line says what
//! it is doing right now in plain words, for how long, and how much it has
//! written, and opens to show the step's detail. Nothing else then looks
//! frozen: a fresh conversation's summary, an engine starting, a long command.
//!
//! The rules, with no drawing: what the line says ([`line`]) from what the
//! screen already knows of the conversation ([`Facts`]). Drawn by
//! `views/chat.rs` for every member's conversation.

use std::time::Duration;

use crate::convo::{Block, Status, ToolLine};

/// What the member is doing, as the line says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// The person's message is in and nothing came back yet: the engine
    /// starting, reading it.
    Reading,
    /// Reasoning.
    Thinking,
    /// Writing its reply.
    Writing,
    /// A step under way: what it does (`verb`, as the step's row says it),
    /// on what, and what it acts on in full, for the line opened.
    Step { verb: String, target: String, detail: Option<String> },
    /// Waiting on a command it left running in the background.
    Background { what: String },
    /// Summarizing its conversation (a fresh one starts after).
    Summarizing,
    /// Waiting on the person to sign in to its engine.
    SignIn { engine: String },
    /// Waiting on the person's approval.
    Approval,
}

/// What the screen knows of a conversation now.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    /// A turn is under way.
    pub working: bool,
    /// How long the turn has run.
    pub working_for: Option<Duration>,
    /// How long the step under way has run.
    pub step_for: Option<Duration>,
    /// How long the summary has run, while one does.
    pub compacting: Option<Duration>,
    /// The conversation's blocks, as drawn.
    pub blocks: &'a [Block],
    /// A command left running in the background it waits on.
    pub background: Option<String>,
    /// The engine it waits for the person to sign in to.
    pub sign_in: Option<String>,
    /// It waits on the person's approval.
    pub approval: bool,
    /// The person's words sent mid-turn and not read yet (steers), and those
    /// left for the turn's end (memos).
    pub steers: usize,
    pub memos: usize,
    /// What the turn has written so far, in tokens.
    pub tokens: Option<u64>,
}

/// The line, as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub doing: Doing,
    /// How long what it says has gone on: the step's time, else the turn's.
    pub elapsed: Duration,
    /// Steps this turn so far.
    pub steps: usize,
    pub tokens: Option<u64>,
    /// The person's words waiting: read at its next step (`steers`) or once
    /// the turn ends (`memos`).
    pub steers: usize,
    pub memos: usize,
}

impl Line {
    /// Whether the line opens to show more.
    pub fn opens(&self) -> bool {
        matches!(&self.doing, Doing::Step { detail: Some(_), .. })
    }
}

/// The step under way: the last of this turn's steps still running.
fn running(blocks: &[Block]) -> Option<&ToolLine> {
    let turn = this_turn(blocks);
    turn.iter().rev().filter_map(|b| if let Block::Steps(lines) = b { Some(lines) } else { None }).flat_map(|lines| lines.iter().rev()).find(|l| l.status == Status::Running && !l.background)
}

/// The blocks since the person's latest message.
fn this_turn(blocks: &[Block]) -> &[Block] {
    let from = blocks.iter().rposition(|b| matches!(b, Block::User(_))).map_or(0, |at| at + 1);
    &blocks[from..]
}

/// The steps this turn so far.
pub fn steps_this_turn(blocks: &[Block]) -> usize {
    this_turn(blocks).iter().map(|b| if let Block::Steps(lines) = b { lines.len() } else { 0 }).sum()
}

/// What the line says, or `None` when there is nothing to say: no turn under
/// way, nothing summarized, nothing waited on. The person comes first: a
/// sign-in or an approval they owe, then a summary, then the step, the
/// background command, the reply, the reasoning — and at the very start, that
/// it is reading their message.
pub fn line(facts: &Facts) -> Option<Line> {
    let turn = facts.working_for.unwrap_or_default();
    let mut elapsed = turn;
    let doing = if let Some(engine) = &facts.sign_in {
        Doing::SignIn { engine: engine.clone() }
    } else if facts.approval {
        Doing::Approval
    } else if let Some(ran) = facts.compacting {
        elapsed = ran;
        Doing::Summarizing
    } else if !facts.working {
        return None;
    } else if let Some(step) = running(facts.blocks) {
        elapsed = facts.step_for.unwrap_or(turn);
        let detail = step.full.clone().or_else(|| step.path.clone()).filter(|d| !d.trim().is_empty() && *d != step.text());
        Doing::Step { verb: step.verb.clone(), target: step.text(), detail }
    } else if let Some(what) = &facts.background {
        Doing::Background { what: what.clone() }
    } else {
        match this_turn(facts.blocks).last() {
            Some(Block::Reply { live: true, .. }) => Doing::Writing,
            Some(Block::Thinking(_)) => Doing::Thinking,
            _ if steps_this_turn(facts.blocks) == 0 && !this_turn(facts.blocks).iter().any(|b| matches!(b, Block::Reply { .. })) => Doing::Reading,
            _ => Doing::Thinking,
        }
    };
    Some(Line { doing, elapsed, steps: steps_this_turn(facts.blocks), tokens: facts.tokens.filter(|t| *t > 0), steers: facts.steers, memos: facts.memos })
}

/// Seconds as the line counts them: `8s`, `1m 05s`, `1h 02m`.
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// Tokens as the line counts them: `812`, `14.1k`, `1.2M`.
pub fn tokens(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}k", n as f64 / 1000.),
        _ => format!("{:.1}M", n as f64 / 1_000_000.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(verb: &str, target: &str, status: Status) -> ToolLine {
        let mut l = ToolLine::new(verb, vec![target.into()], false);
        l.status = status;
        l
    }

    fn working(blocks: &[Block]) -> Facts<'_> {
        Facts { working: true, working_for: Some(Duration::from_secs(5)), blocks, ..Facts::default() }
    }

    #[test]
    fn right_after_the_person_speaks_it_says_it_is_reading_their_message() {
        let blocks = [Block::Reply { text: "earlier".into(), first: true, live: false, worked: None }, Block::User("make the menu calmer".into())];
        let got = line(&Facts { tokens: Some(14_100), ..working(&blocks) }).unwrap();
        assert_eq!(got.doing, Doing::Reading);
        assert_eq!((got.elapsed, got.steps, got.tokens), (Duration::from_secs(5), 0, Some(14_100)));
        assert_eq!(line(&Facts { working: false, ..working(&blocks) }), None, "nothing under way: no line");
    }

    #[test]
    fn a_step_under_way_names_what_it_does_counts_its_own_time_and_opens_to_its_detail() {
        let mut run = step("Ran", "npm run build", Status::Running);
        run.full = Some("npm run build && npx playwright test --project=all".into());
        let blocks = [Block::User("go".into()), Block::Steps(vec![step("Read", "menu.tsx", Status::Done), run])];
        let got = line(&Facts { step_for: Some(Duration::from_secs(12)), ..working(&blocks) }).unwrap();
        assert_eq!(got.doing, Doing::Step { verb: "Ran".into(), target: "npm run build".into(), detail: Some("npm run build && npx playwright test --project=all".into()) });
        assert_eq!((got.elapsed, got.steps), (Duration::from_secs(12), 2));
        assert!(got.opens());
    }

    #[test]
    fn between_steps_it_says_it_thinks_or_writes() {
        let blocks = [Block::User("go".into()), Block::Steps(vec![step("Read", "a.rs", Status::Done)]), Block::Thinking("hmm".into())];
        assert_eq!(line(&working(&blocks)).unwrap().doing, Doing::Thinking);
        let blocks = [Block::User("go".into()), Block::Reply { text: "Here".into(), first: false, live: true, worked: None }];
        assert_eq!(line(&working(&blocks)).unwrap().doing, Doing::Writing);
        let blocks = [Block::User("go".into()), Block::Steps(vec![step("Read", "a.rs", Status::Done)])];
        assert_eq!(line(&working(&blocks)).unwrap().doing, Doing::Thinking, "a step done, the next not begun");
    }

    #[test]
    fn a_summary_shows_even_with_no_turn_and_the_person_owed_comes_first() {
        let blocks = [Block::User("go".into())];
        let summary = line(&Facts { working: false, compacting: Some(Duration::from_secs(30)), ..working(&blocks) }).unwrap();
        assert_eq!((summary.doing, summary.elapsed), (Doing::Summarizing, Duration::from_secs(30)));
        let signin = line(&Facts { sign_in: Some("Claude".into()), compacting: Some(Duration::from_secs(1)), ..working(&blocks) }).unwrap();
        assert_eq!(signin.doing, Doing::SignIn { engine: "Claude".into() });
        assert_eq!(line(&Facts { approval: true, ..working(&blocks) }).unwrap().doing, Doing::Approval);
        let bg = line(&Facts { background: Some("dev server".into()), ..working(&blocks) }).unwrap();
        assert_eq!(bg.doing, Doing::Background { what: "dev server".into() });
    }

    #[test]
    fn the_persons_waiting_words_go_with_the_line() {
        let blocks = [Block::User("go".into())];
        let got = line(&Facts { steers: 1, memos: 2, ..working(&blocks) }).unwrap();
        assert_eq!((got.steers, got.memos), (1, 2));
    }

    #[test]
    fn time_and_tokens_read_short() {
        assert_eq!(clock(Duration::from_secs(8)), "8s");
        assert_eq!(clock(Duration::from_secs(65)), "1m 05s");
        assert_eq!(clock(Duration::from_secs(3720)), "1h 02m");
        assert_eq!(tokens(812), "812");
        assert_eq!(tokens(14_100), "14.1k");
        assert_eq!(tokens(1_234_567), "1.2M");
    }
}
