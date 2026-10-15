//! Pairing a phone (the equipe app) from the window, with no terminal: the
//! rules behind the sheet `views/pair.rs` draws. The daemon does the pairing —
//! on its own machine, which may not be this Mac — and tells this window
//! alone what happens (`Command::PairStart` … `Event::PairOffer` …), so what
//! is here is only what the sheet shows and what to ask next.
//!
//! A code is a secret. It lives in [`Stage::Showing`] while the sheet is open
//! and nowhere else: closing, a phone pairing, an error or a new code drops
//! it, and nothing here logs or keeps it.

use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Event, PairEnd, PairedDevice};

/// A code that runs out while the sheet is open is replaced this many times
/// (5 minutes each) before the sheet asks the person to.
pub const REFRESHES: u32 = 3;
/// With no word from the daemon this long after asking, it doesn't know how
/// to pair phones: it's older than this app.
pub const NO_ANSWER: Duration = Duration::from_secs(8);
/// How long "연결됨" stays before the sheet closes by itself.
pub const PAIRED_FOR: Duration = Duration::from_millis(1800);

/// Why the sheet stopped, as it explains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The person said no to the phone that asked.
    Refused,
    /// The code ran out, and was replaced as often as it is.
    Expired,
    /// Another screen or `equipe pair` has a code open.
    Busy,
    /// The relay couldn't be reached.
    Relay,
    /// The daemon's phone link isn't running.
    Unavailable,
    /// The daemon never answered: it's older than this app.
    TooOld,
    /// The window lost the daemon.
    Disconnected,
    Failed,
}

impl From<PairEnd> for Problem {
    fn from(end: PairEnd) -> Self {
        match end {
            PairEnd::Refused => Problem::Refused,
            PairEnd::Expired => Problem::Expired,
            PairEnd::Busy => Problem::Busy,
            PairEnd::Relay => Problem::Relay,
            PairEnd::Unavailable => Problem::Unavailable,
            PairEnd::Failed => Problem::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// A code was asked for; none yet.
    Opening { since: Instant },
    /// The code, on screen until `until`.
    Showing { url: String, until: Instant },
    /// A phone brought the code and asks to pair: yes or no.
    Asked { name: String, platform: String },
    /// Said yes; the daemon is finishing.
    Approving { name: String },
    /// Paired: shown a moment, then the sheet closes.
    Paired { name: String, at: Instant },
    /// Stopped: why, and the daemon's words when it said some.
    Ended { problem: Problem, message: String },
}

/// The phones paired with the daemon's machine, as it last said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Devices {
    pub devices: Vec<PairedDevice>,
    pub relay: String,
    pub connected: bool,
}

#[derive(Debug)]
pub struct Pairing {
    /// The sheet is on screen.
    pub open: bool,
    pub stage: Stage,
    /// Codes replaced since the sheet opened (or since "새 코드").
    refreshes: u32,
    /// The daemon has answered a pairing command: it knows them.
    pub answered: bool,
    /// The list, once the daemon said it; when it was last asked for.
    pub devices: Option<Devices>,
    pub devices_asked: Option<Instant>,
    /// "링크 복사" was pressed, to say "복사됨" a moment.
    pub copied: Option<Instant>,
    /// Bumped on each new stage, for the sheet's animations.
    pub turn: u64,
}

impl Default for Pairing {
    fn default() -> Self {
        Self { open: false, stage: Stage::Opening { since: Instant::now() }, refreshes: 0, answered: false, devices: None, devices_asked: None, copied: None, turn: 0 }
    }
}

impl Pairing {
    fn set(&mut self, stage: Stage) {
        self.stage = stage;
        self.copied = None;
        self.turn += 1;
    }

    /// The sheet opens: the list and a code are asked for.
    pub fn open(&mut self, now: Instant) -> Vec<Command> {
        self.open = true;
        self.refreshes = 0;
        self.set(Stage::Opening { since: now });
        self.devices_asked = Some(now);
        vec![Command::Devices, Command::PairStart]
    }

    /// The sheet closes (Esc, 닫기, or by itself once paired): the code, if
    /// any, stops working, and leaves this window's memory. A phone left
    /// asking is told no.
    pub fn close(&mut self) -> Vec<Command> {
        if !self.open {
            return vec![];
        }
        self.open = false;
        let send = match &self.stage {
            Stage::Opening { .. } | Stage::Showing { .. } => vec![Command::PairCancel],
            Stage::Asked { .. } => vec![Command::PairAnswer { approve: false }],
            Stage::Approving { .. } | Stage::Paired { .. } | Stage::Ended { .. } => vec![],
        };
        self.set(Stage::Ended { problem: Problem::Failed, message: String::new() });
        send
    }

    /// "다시 시도" / "새 코드": a fresh code, the count of replacements anew.
    pub fn retry(&mut self, now: Instant) -> Vec<Command> {
        self.refreshes = 0;
        self.set(Stage::Opening { since: now });
        vec![Command::PairStart]
    }

    /// The person's answer to the phone that asked.
    pub fn answer(&mut self, approve: bool) -> Vec<Command> {
        let Stage::Asked { name, .. } = &self.stage else { return vec![] };
        if approve {
            let name = name.clone();
            self.set(Stage::Approving { name });
        }
        vec![Command::PairAnswer { approve }]
    }

    /// What the daemon said. True when the window has something new to draw.
    pub fn on_event(&mut self, event: &Event, now: Instant) -> bool {
        match event {
            Event::Devices { devices, relay, connected } => {
                self.answered = true;
                self.devices = Some(Devices { devices: devices.clone(), relay: relay.clone(), connected: *connected });
                true
            }
            // Late words for a sheet already closed are dropped — a code above all.
            _ if !self.open => false,
            Event::PairOffer { url, seconds } => {
                self.answered = true;
                self.set(Stage::Showing { url: url.clone(), until: now + Duration::from_secs(*seconds) });
                true
            }
            Event::PairAsked { name, platform } => {
                self.answered = true;
                self.set(Stage::Asked { name: name.clone(), platform: platform.clone() });
                true
            }
            Event::Paired { name } => {
                self.answered = true;
                self.set(Stage::Paired { name: name.clone(), at: now });
                true
            }
            Event::PairEnded { why, message } => {
                self.answered = true;
                // The old code's end, told while a new one is on its way.
                if *why == PairEnd::Expired && matches!(self.stage, Stage::Opening { .. }) {
                    return false;
                }
                if *why == PairEnd::Expired && self.refreshes < REFRESHES {
                    self.refreshes += 1;
                    self.set(Stage::Opening { since: now });
                    return true;
                }
                self.set(Stage::Ended { problem: (*why).into(), message: message.clone() });
                true
            }
            _ => false,
        }
    }

    /// The window lost the daemon: the code died with the connection.
    pub fn disconnected(&mut self) {
        if self.open && !matches!(self.stage, Stage::Paired { .. } | Stage::Ended { .. }) {
            self.set(Stage::Ended { problem: Problem::Disconnected, message: String::new() });
        }
    }

    /// The clock moved: what to ask the daemon now. A code past its time is
    /// replaced (a few times); a daemon that never answers is too old; a
    /// pairing shown done closes the sheet.
    pub fn tick(&mut self, now: Instant) -> Vec<Command> {
        if !self.open {
            return vec![];
        }
        match &self.stage {
            Stage::Opening { since } if !self.answered && now >= *since + NO_ANSWER => {
                self.set(Stage::Ended { problem: Problem::TooOld, message: String::new() });
                vec![]
            }
            Stage::Showing { until, .. } if now >= *until => {
                if self.refreshes < REFRESHES {
                    self.refreshes += 1;
                    self.set(Stage::Opening { since: now });
                    vec![Command::PairStart]
                } else {
                    self.set(Stage::Ended { problem: Problem::Expired, message: String::new() });
                    vec![Command::PairCancel]
                }
            }
            Stage::Paired { at, .. } if now >= *at + PAIRED_FOR => self.close(),
            _ => vec![],
        }
    }

    /// Seconds the code on screen still works.
    pub fn seconds_left(&self, now: Instant) -> Option<u64> {
        match &self.stage {
            Stage::Showing { until, .. } => Some(until.saturating_duration_since(now).as_secs_f32().ceil() as u64),
            _ => None,
        }
    }

    /// Whether a code that runs out is replaced by itself.
    pub fn renews(&self) -> bool {
        self.refreshes < REFRESHES
    }

    /// The code on screen, for the QR and "링크 복사".
    pub fn url(&self) -> Option<&str> {
        match &self.stage {
            Stage::Showing { url, .. } => Some(url),
            _ => None,
        }
    }

    /// The list was asked for long enough ago, and never came: an older daemon.
    pub fn devices_unknown(&self, now: Instant) -> bool {
        self.devices.is_none() && self.devices_asked.is_some_and(|at| now >= at + NO_ANSWER)
    }
}

/// `m:ss`.
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A QR code of `text`: its width in modules and which are dark, row by row.
/// The quiet zone around it is the drawer's.
pub fn qr(text: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    Some((width, code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect()))
}

/// The dark modules as runs along each row — (row, first column, length) —
/// so a code is a few hundred rectangles, not thousands.
pub fn runs(width: usize, dark: &[bool]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (row, line) in dark.chunks(width).enumerate() {
        let mut col = 0;
        while col < line.len() {
            if line[col] {
                let start = col;
                while col < line.len() && line[col] {
                    col += 1;
                }
                out.push((row, start, col - start));
            } else {
                col += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn showing(p: &mut Pairing, now: Instant) {
        assert!(p.on_event(&Event::PairOffer { url: "equipe://pair?s=secret".into(), seconds: 300 }, now));
    }

    #[test]
    fn opening_asks_for_the_list_and_a_code_and_the_code_shows_with_its_time() {
        let now = Instant::now();
        let mut p = Pairing::default();
        assert_eq!(p.open(now), vec![Command::Devices, Command::PairStart]);
        assert!(matches!(p.stage, Stage::Opening { .. }) && p.url().is_none());
        showing(&mut p, now);
        assert_eq!(p.url(), Some("equipe://pair?s=secret"));
        assert_eq!(p.seconds_left(now), Some(300));
        assert_eq!(p.seconds_left(now + Duration::from_millis(1500)), Some(299), "a second begun counts");
        assert_eq!(clock(299), "4:59");
        assert_eq!(clock(5), "0:05");
    }

    #[test]
    fn a_phone_asks_the_person_says_yes_and_the_sheet_closes_by_itself() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        showing(&mut p, now);
        p.on_event(&Event::PairAsked { name: "Abel's iPhone".into(), platform: "ios".into() }, now);
        assert!(p.url().is_none(), "the code leaves the screen once a phone has it");
        assert_eq!(p.answer(true), vec![Command::PairAnswer { approve: true }]);
        assert_eq!(p.stage, Stage::Approving { name: "Abel's iPhone".into() });
        assert!(p.answer(true).is_empty(), "one answer");
        p.on_event(&Event::Paired { name: "Abel's iPhone".into() }, now);
        assert!(p.tick(now + Duration::from_millis(500)).is_empty() && p.open, "연결됨 stays a moment");
        assert!(p.tick(now + PAIRED_FOR).is_empty(), "nothing to cancel once paired");
        assert!(!p.open);
    }

    #[test]
    fn a_no_is_said_and_shown() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        p.on_event(&Event::PairAsked { name: "x".into(), platform: "ios".into() }, now);
        assert_eq!(p.answer(false), vec![Command::PairAnswer { approve: false }]);
        p.on_event(&Event::PairEnded { why: PairEnd::Refused, message: String::new() }, now);
        assert!(matches!(p.stage, Stage::Ended { problem: Problem::Refused, .. }));
        assert_eq!(p.retry(now), vec![Command::PairStart]);
    }

    #[test]
    fn a_code_that_runs_out_is_replaced_a_few_times_then_the_person_is_asked() {
        let mut now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        for _ in 0..REFRESHES {
            showing(&mut p, now);
            assert!(p.renews());
            assert!(p.tick(now + Duration::from_secs(299)).is_empty());
            now += Duration::from_secs(300);
            assert_eq!(p.tick(now), vec![Command::PairStart]);
            assert!(p.url().is_none(), "the old code is gone at once");
            // The daemon's word that the old one ran out, late: no matter.
            assert!(!p.on_event(&Event::PairEnded { why: PairEnd::Expired, message: "old".into() }, now));
            assert!(matches!(p.stage, Stage::Opening { .. }));
        }
        showing(&mut p, now);
        assert!(!p.renews());
        assert_eq!(p.tick(now + Duration::from_secs(300)), vec![Command::PairCancel]);
        assert!(matches!(p.stage, Stage::Ended { problem: Problem::Expired, .. }));
        // "새 코드" starts the count over.
        assert_eq!(p.retry(now), vec![Command::PairStart]);
        assert!(p.renews());
    }

    #[test]
    fn the_daemon_saying_a_code_ran_out_replaces_it_too() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        showing(&mut p, now);
        assert!(p.on_event(&Event::PairEnded { why: PairEnd::Expired, message: String::new() }, now));
        assert!(matches!(p.stage, Stage::Opening { .. }));
    }

    #[test]
    fn closing_stops_the_code_and_forgets_it_and_late_words_are_dropped() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        assert_eq!(p.close(), vec![Command::PairCancel], "closed while the code was coming");
        p.open(now);
        showing(&mut p, now);
        assert_eq!(p.close(), vec![Command::PairCancel]);
        assert!(p.url().is_none() && !p.open);
        assert!(p.close().is_empty(), "closing twice says nothing");
        // A code arriving after the sheet closed never lands.
        assert!(!p.on_event(&Event::PairOffer { url: "equipe://late".into(), seconds: 300 }, now));
        assert!(p.url().is_none());
        // A phone left asking is told no.
        p.open(now);
        p.on_event(&Event::PairAsked { name: "x".into(), platform: "ios".into() }, now);
        assert_eq!(p.close(), vec![Command::PairAnswer { approve: false }]);
        // The list is kept whether the sheet is open or not.
        assert!(p.on_event(&Event::Devices { devices: vec![], relay: "r".into(), connected: false }, now));
        assert!(p.devices.is_some());
    }

    #[test]
    fn a_daemon_that_never_answers_is_too_old_and_one_that_did_isnt() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        assert!(p.tick(now + Duration::from_secs(3)).is_empty());
        assert!(!p.devices_unknown(now + Duration::from_secs(3)));
        p.tick(now + NO_ANSWER);
        assert!(matches!(p.stage, Stage::Ended { problem: Problem::TooOld, .. }));
        assert!(p.devices_unknown(now + NO_ANSWER));
        // One that answered the list may take its time with the relay.
        let mut p = Pairing::default();
        p.open(now);
        p.on_event(&Event::Devices { devices: vec![], relay: String::new(), connected: false }, now);
        assert!(p.tick(now + NO_ANSWER * 3).is_empty());
        assert!(matches!(p.stage, Stage::Opening { .. }));
    }

    #[test]
    fn errors_are_kept_with_their_words_and_losing_the_daemon_ends_the_code() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.open(now);
        p.on_event(&Event::PairEnded { why: PairEnd::Relay, message: "couldn't reach the relay".into() }, now);
        assert_eq!(p.stage, Stage::Ended { problem: Problem::Relay, message: "couldn't reach the relay".into() });
        p.retry(now);
        showing(&mut p, now);
        p.disconnected();
        assert!(matches!(p.stage, Stage::Ended { problem: Problem::Disconnected, .. }) && p.url().is_none());
    }

    #[test]
    fn a_qr_code_is_square_and_drawn_as_runs_of_its_dark_modules() {
        let (width, dark) = qr("equipe://pair?r=https%3A%2F%2Frelay.equipe.im&d=0123456789abcdef0123456789abcdef&k=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&s=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB&n=studio").unwrap();
        assert_eq!(dark.len(), width * width);
        assert_eq!((width - 21) % 4, 0, "a QR code's width is 21 + 4 per version");
        // The finder pattern's top row: seven dark modules from the corner.
        assert!(dark[..7].iter().all(|d| *d) && !dark[7]);
        let runs = runs(width, &dark);
        assert_eq!(runs[0], (0, 0, 7));
        assert_eq!(runs.iter().map(|r| r.2).sum::<usize>(), dark.iter().filter(|d| **d).count(), "every dark module, once");
        assert!(runs.len() < dark.iter().filter(|d| **d).count() / 2, "far fewer runs than modules");
        assert_eq!(super::runs(3, &[true, true, false, false, true, true, true, false, true]), vec![(0, 0, 2), (1, 1, 2), (2, 0, 1), (2, 2, 1)]);
    }
}
