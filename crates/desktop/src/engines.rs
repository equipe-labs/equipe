//! Settings' AI 플랫폼: the engines members work on, from a registry, and the
//! rules behind each one's card that `views/engines.rs` draws. The daemon
//! asks each engine's own command-line tool where it stands and runs its
//! sign-in on its own machine — which may not be this Mac — and tells every
//! screen (`Command::Engines`, `Command::SignIn` … `Event::Engines`,
//! `Event::SignIn`), so what is here is only what a card shows and what to
//! ask next.
//!
//! A platform is an entry in [`PLATFORMS`]: the cards, their words and their
//! buttons follow from it, so one more engine is one more entry.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Engine, EngineState, EngineStatus, Event, PlanUsage, SignInFailure, SignInStep};
use equipe_core::t;

use crate::icons::Icon;

/// How the person signs a platform in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignsIn {
    /// From the card: the daemon runs the tool's own sign-in on its machine.
    FromApp,
    /// The person runs `command` in a terminal on the daemon's machine; the
    /// card opens one on this Mac, and says what to run on another.
    InTerminal { command: &'static str },
}

/// One AI platform members can work on.
#[derive(Debug, Clone, Copy)]
pub struct Platform {
    pub engine: Engine,
    /// Its own name, the same in every language.
    pub name: &'static str,
    pub mark: Icon,
    /// The mark's tile, `0xRRGGBB`.
    pub tint: u32,
    /// Its command-line tool, as the person types it.
    pub tool: &'static str,
    pub signs_in: SignsIn,
    /// The line that installs the tool, and where its own guide is.
    pub install: &'static str,
    pub guide: &'static str,
    /// Whose plan it draws on and who signs it in, in a line.
    pub who: fn() -> &'static str,
    /// What connecting it adds, in a line, while it isn't.
    pub pitch: fn() -> &'static str,
    /// Where its plan's remaining usage comes from today, if anywhere.
    pub usage: Usage,
}

/// Where a platform's remaining usage is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    /// The plan's windows, as Claude Code reports them after a turn.
    ClaudePlan,
    /// Not read yet: the row says it will be.
    Later,
}

/// The platforms, in the order the page shows them.
pub static PLATFORMS: [Platform; 2] = [
    Platform {
        engine: Engine::Claude,
        name: "Claude Code",
        mark: Icon::ClaudeMark,
        tint: 0xD97757,
        tool: "claude",
        signs_in: SignsIn::InTerminal { command: "claude auth login" },
        install: "curl -fsSL https://claude.ai/install.sh | bash",
        guide: "https://code.claude.com/docs/en/setup",
        who: || t!("engines.who_claude"),
        pitch: || t!("engines.pitch_claude"),
        usage: Usage::ClaudePlan,
    },
    Platform {
        engine: Engine::Codex,
        name: "Codex",
        mark: Icon::OpenAiMark,
        tint: 0x0D0D0D,
        tool: "codex",
        signs_in: SignsIn::FromApp,
        install: "npm install -g @openai/codex",
        guide: "https://developers.openai.com/codex/cli",
        who: || t!("engines.who_codex"),
        pitch: || t!("engines.pitch_codex"),
        usage: Usage::Later,
    },
];

pub fn platform(engine: Engine) -> &'static Platform {
    PLATFORMS.iter().find(|p| p.engine == engine).expect("every engine has a platform entry")
}

/// With no word from the daemon this long after asking, it doesn't know the
/// command: it's older than this app.
pub const NO_ANSWER: Duration = Duration::from_secs(8);
/// While the person signs Claude Code in in a terminal, the card looks again this often…
pub const TERMINAL_POLL: Duration = Duration::from_secs(4);
/// …for at most this long.
pub const TERMINAL_WATCH: Duration = Duration::from_secs(10 * 60);

/// Why a card's sign-in stopped, as it explains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// What the daemon said.
    Daemon(SignInFailure),
    /// The daemon never answered: it's older than this app.
    NoAnswer,
    /// The window lost the daemon.
    Lost,
}

/// Where one card's sign-in (or sign-out) stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Card {
    #[default]
    Idle,
    /// Asked the daemon to start; nothing back yet.
    Starting { since: Instant },
    /// The browser opened on the daemon's machine; `url` to open by hand.
    Browser { url: Option<String> },
    /// Enter `code` at `url`, in any browser.
    Code { url: String, code: String },
    /// Another screen started it.
    Elsewhere,
    Failed { problem: Problem, detail: String },
    /// "로그아웃" pressed: asked once more on the card.
    ConfirmSignOut,
    SigningOut,
}

impl Card {
    /// A sign-in is on its way: the card shows it and offers 취소.
    pub fn under_way(&self) -> bool {
        matches!(self, Card::Starting { .. } | Card::Browser { .. } | Card::Code { .. } | Card::Elsewhere)
    }
}

/// What an event asks of the window besides drawing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effect {
    pub changed: bool,
    /// Open this page in this Mac's browser (the code's page, for the screen that asked).
    pub open: Option<String>,
}

#[derive(Debug, Default)]
pub struct Engines {
    /// Where each engine stands, as the daemon last said.
    pub statuses: Option<Vec<EngineStatus>>,
    /// When the list was asked for while none had come.
    pub asked: Option<Instant>,
    cards: HashMap<Engine, Card>,
    /// "복사됨" for a moment after the code or the link was copied.
    pub copied: Option<(Engine, Instant)>,
    /// Since when the person has been signing Claude Code in in a terminal.
    pub terminal: Option<Instant>,
    /// Bumped on each card's change, for its animation.
    pub turn: u64,
}

impl Engines {
    pub fn card(&self, engine: Engine) -> &Card {
        static IDLE: Card = Card::Idle;
        self.cards.get(&engine).unwrap_or(&IDLE)
    }

    fn set(&mut self, engine: Engine, card: Card) {
        if self.card(engine) != &card {
            self.cards.insert(engine, card);
            self.turn += 1;
        }
    }

    pub fn status(&self, engine: Engine) -> Option<&EngineStatus> {
        self.statuses.as_ref()?.iter().find(|s| s.engine == engine)
    }

    /// The page opens: where the engines stand is asked for.
    pub fn open(&mut self, now: Instant) -> Vec<Command> {
        if self.statuses.is_none() && self.asked.is_none() {
            self.asked = Some(now);
        }
        vec![Command::Engines]
    }

    /// The daemon never said: the page explains rather than waiting on.
    pub fn no_answer(&self, now: Instant) -> bool {
        self.statuses.is_none() && self.asked.is_some_and(|at| now.duration_since(at) >= NO_ANSWER)
    }

    /// "로그인": the daemon runs the tool's sign-in; `remote` (the daemon is on
    /// another Mac, the browser here) asks for a code instead of its browser.
    pub fn sign_in(&mut self, engine: Engine, remote: bool, now: Instant) -> Vec<Command> {
        if self.card(engine).under_way() || platform(engine).signs_in != SignsIn::FromApp {
            return vec![];
        }
        self.set(engine, Card::Starting { since: now });
        vec![Command::SignIn { engine, device: remote }]
    }

    /// "취소": the daemon stops the tool.
    pub fn cancel(&mut self, engine: Engine) -> Vec<Command> {
        if !self.card(engine).under_way() {
            return vec![];
        }
        self.set(engine, Card::Idle);
        vec![Command::SignInCancel { engine }]
    }

    /// "로그아웃" asks first; "유지" puts it back.
    pub fn ask_sign_out(&mut self, engine: Engine) {
        self.set(engine, Card::ConfirmSignOut);
    }

    pub fn keep(&mut self, engine: Engine) {
        if matches!(self.card(engine), Card::ConfirmSignOut | Card::Failed { .. }) {
            self.set(engine, Card::Idle);
        }
    }

    pub fn sign_out(&mut self, engine: Engine) -> Vec<Command> {
        if self.card(engine) != &Card::ConfirmSignOut {
            return vec![];
        }
        self.set(engine, Card::SigningOut);
        vec![Command::SignOut { engine }]
    }

    pub fn on_event(&mut self, event: &Event) -> Effect {
        match event {
            Event::Engines { engines } => {
                self.statuses = Some(engines.clone());
                for status in engines {
                    let card = self.card(status.engine).clone();
                    match card {
                        Card::SigningOut => self.set(status.engine, Card::Idle),
                        Card::Idle if status.signing_in => self.set(status.engine, Card::Elsewhere),
                        Card::Elsewhere if !status.signing_in => self.set(status.engine, Card::Idle),
                        _ => {}
                    }
                    if status.engine == Engine::Claude && status.state == EngineState::SignedIn {
                        self.terminal = None;
                    }
                }
                Effect { changed: true, open: None }
            }
            Event::SignIn { engine, step } => {
                let engine = *engine;
                let asked = matches!(self.card(engine), Card::Starting { .. });
                let mut open = None;
                match step {
                    SignInStep::Browser { url } => {
                        let url = url.clone().or_else(|| match self.card(engine) {
                            Card::Browser { url } => url.clone(),
                            _ => None,
                        });
                        self.set(engine, Card::Browser { url });
                    }
                    SignInStep::DeviceCode { url, code } => {
                        // The screen that asked opens the page; the others show the code.
                        if asked {
                            open = Some(url.clone());
                        }
                        self.set(engine, Card::Code { url: url.clone(), code: code.clone() });
                    }
                    SignInStep::Done | SignInStep::Cancelled => self.set(engine, Card::Idle),
                    SignInStep::Failed { why, detail } => {
                        // Another screen's no about an engine nobody here is signing in.
                        if self.card(engine) == &Card::Idle {
                            return Effect::default();
                        }
                        self.set(engine, Card::Failed { problem: Problem::Daemon(*why), detail: detail.clone() });
                    }
                }
                Effect { changed: true, open }
            }
            _ => Effect::default(),
        }
    }

    /// Time passes: a start the daemon never answers ends.
    pub fn tick(&mut self, now: Instant) -> bool {
        let silent: Vec<Engine> = self.cards.iter().filter(|(_, c)| matches!(c, Card::Starting { since } if now.duration_since(*since) >= NO_ANSWER)).map(|(e, _)| *e).collect();
        for engine in &silent {
            self.set(*engine, Card::Failed { problem: Problem::NoAnswer, detail: String::new() });
        }
        if self.terminal.is_some_and(|since| now.duration_since(since) >= TERMINAL_WATCH) {
            self.terminal = None;
        }
        !silent.is_empty()
    }

    /// The window lost the daemon: what was on its way stops, said so.
    pub fn disconnected(&mut self) {
        let going: Vec<Engine> = self.cards.iter().filter(|(_, c)| c.under_way() || **c == Card::SigningOut).map(|(e, _)| *e).collect();
        for engine in going {
            self.set(engine, Card::Failed { problem: Problem::Lost, detail: String::new() });
        }
        self.asked = None;
    }

    /// Attached (again, maybe to another daemon): what was known is stale.
    pub fn attached(&mut self) {
        self.statuses = None;
        self.asked = None;
    }
}

/// How a status reads at a glance, and whether it is good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ready,
    Needs,
    Unknown,
}

/// The status in a few words: "로그인됨 · Max", "로그인 필요", "codex CLI 설치 필요".
pub fn headline(status: &EngineStatus) -> (Tone, String) {
    let tool = platform(status.engine).tool;
    match status.state {
        EngineState::SignedIn if status.per_token => (Tone::Ready, t!("engines.per_token").to_owned()),
        EngineState::SignedIn => match &status.plan {
            Some(plan) => (Tone::Ready, t!("engines.signed_in_plan", plan = title_case(plan))),
            None => (Tone::Ready, t!("engines.signed_in").to_owned()),
        },
        EngineState::SignedOut => (Tone::Needs, t!("engines.signed_out").to_owned()),
        EngineState::NotInstalled => (Tone::Needs, t!("engines.not_installed", tool = tool)),
        EngineState::Unknown => (Tone::Unknown, t!("engines.unknown").to_owned()),
    }
}

/// `max` → `Max`.
pub fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Who is on it: names in this squad, `squad/name` elsewhere.
pub fn members_line(status: &EngineStatus, squad: &str) -> Option<String> {
    let names: Vec<String> = status.members.iter().map(|m| if m.squad == squad { m.member.clone() } else { format!("{}/{}", m.squad, m.member) }).collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The plan's windows, as the usage row says them: "5시간 42% · 1시간 10분 뒤 초기화".
pub fn plan_usage(plan: &PlanUsage, now_unix: i64) -> Vec<(String, u8)> {
    [(t!("engines.window_5h"), plan.five_hour), (t!("engines.window_week"), plan.seven_day)]
        .into_iter()
        .filter_map(|(name, window)| {
            let w = window?;
            let mins = ((w.resets_at - now_unix).max(0) + 59) / 60;
            let when = match mins {
                m if m >= 24 * 60 => t!("engines.resets_in_d", d = m / (24 * 60)),
                m if m >= 60 => t!("engines.resets_in_hm", h = m / 60, m = m % 60),
                m => t!("engines.resets_in_m", m = m),
            };
            Some((t!("engines.usage_window", window = name, percent = w.used_percent, when = when), w.used_percent.min(100)))
        })
        .collect()
}

/// A failed sign-in, in a line the person can act on.
pub fn problem_words(problem: Problem, host: &str) -> String {
    match problem {
        Problem::Daemon(SignInFailure::DeviceCodeOff) => t!("engines.fail_device_off").to_owned(),
        Problem::Daemon(SignInFailure::Expired) => t!("engines.fail_expired").to_owned(),
        Problem::Daemon(SignInFailure::NotInstalled) => t!("engines.fail_not_installed", host = host),
        Problem::Daemon(SignInFailure::Unsupported | SignInFailure::Failed) => t!("engines.fail_other").to_owned(),
        Problem::NoAnswer => t!("engines.fail_no_answer").to_owned(),
        Problem::Lost => t!("engines.fail_lost").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::{MemberRef, PlanWindow};

    fn status(engine: Engine, state: EngineState) -> EngineStatus {
        EngineStatus::new(engine, state)
    }

    #[test]
    fn every_engine_has_one_platform_entry_with_words_of_its_own() {
        for engine in [Engine::Claude, Engine::Codex] {
            assert_eq!(PLATFORMS.iter().filter(|p| p.engine == engine).count(), 1, "{engine:?}");
        }
        for p in &PLATFORMS {
            assert!(!(p.who)().is_empty() && !(p.pitch)().is_empty() && !p.install.is_empty() && p.guide.starts_with("https://"));
        }
        assert_eq!(platform(Engine::Codex).signs_in, SignsIn::FromApp);
        assert_eq!(platform(Engine::Claude).signs_in, SignsIn::InTerminal { command: "claude auth login" }, "Claude Code is the person's own login");
        assert_eq!((platform(Engine::Codex).pitch)(), "Codex를 연결하면 두 구독을 나눠 써서 한도에 덜 걸려요.");
    }

    #[test]
    fn signing_in_from_this_mac_opens_its_browser_and_from_another_shows_a_code() {
        let mut e = Engines::default();
        let now = Instant::now();
        assert_eq!(e.sign_in(Engine::Codex, false, now), vec![Command::SignIn { engine: Engine::Codex, device: false }]);
        assert!(e.sign_in(Engine::Codex, false, now).is_empty(), "one at a time");
        e.on_event(&Event::SignIn { engine: Engine::Codex, step: SignInStep::Browser { url: None } });
        e.on_event(&Event::SignIn { engine: Engine::Codex, step: SignInStep::Browser { url: Some("https://a/b".into()) } });
        assert_eq!(e.card(Engine::Codex), &Card::Browser { url: Some("https://a/b".into()) });
        e.on_event(&Event::SignIn { engine: Engine::Codex, step: SignInStep::Done });
        assert_eq!(e.card(Engine::Codex), &Card::Idle);

        assert_eq!(e.sign_in(Engine::Codex, true, now), vec![Command::SignIn { engine: Engine::Codex, device: true }]);
        let code = Event::SignIn { engine: Engine::Codex, step: SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "AB-12".into() } };
        assert_eq!(e.on_event(&code).open.as_deref(), Some("https://auth.openai.com/codex/device"), "the screen that asked opens the page");
        assert_eq!(e.card(Engine::Codex), &Card::Code { url: "https://auth.openai.com/codex/device".into(), code: "AB-12".into() });
        assert_eq!(e.cancel(Engine::Codex), vec![Command::SignInCancel { engine: Engine::Codex }]);
        assert_eq!(e.card(Engine::Codex), &Card::Idle);
        assert!(e.cancel(Engine::Codex).is_empty());
    }

    #[test]
    fn another_screens_sign_in_shows_here_without_opening_anything() {
        let mut e = Engines::default();
        let code = Event::SignIn { engine: Engine::Codex, step: SignInStep::DeviceCode { url: "https://x".into(), code: "C".into() } };
        assert_eq!(e.on_event(&code).open, None);
        assert!(e.card(Engine::Codex).under_way());
        let mut fresh = Engines::default();
        let mut codex = status(Engine::Codex, EngineState::SignedOut);
        codex.signing_in = true;
        fresh.on_event(&Event::Engines { engines: vec![codex.clone()] });
        assert_eq!(fresh.card(Engine::Codex), &Card::Elsewhere);
        codex.signing_in = false;
        fresh.on_event(&Event::Engines { engines: vec![codex] });
        assert_eq!(fresh.card(Engine::Codex), &Card::Idle);
        // A no for nobody's sign-in here says nothing.
        assert_eq!(fresh.on_event(&Event::SignIn { engine: Engine::Claude, step: SignInStep::Failed { why: SignInFailure::Unsupported, detail: String::new() } }), Effect::default());
    }

    #[test]
    fn a_failure_a_silent_daemon_or_a_lost_one_ends_it_with_a_retry() {
        let mut e = Engines::default();
        let now = Instant::now();
        e.sign_in(Engine::Codex, true, now);
        e.on_event(&Event::SignIn { engine: Engine::Codex, step: SignInStep::Failed { why: SignInFailure::DeviceCodeOff, detail: "not enabled".into() } });
        assert_eq!(e.card(Engine::Codex), &Card::Failed { problem: Problem::Daemon(SignInFailure::DeviceCodeOff), detail: "not enabled".into() });
        assert!(problem_words(Problem::Daemon(SignInFailure::DeviceCodeOff), "studio").contains("기기 코드"));
        assert_eq!(e.sign_in(Engine::Codex, true, now).len(), 1, "다시 시도 starts again");

        assert!(!e.tick(now + Duration::from_secs(2)));
        assert!(e.tick(now + NO_ANSWER));
        assert_eq!(e.card(Engine::Codex), &Card::Failed { problem: Problem::NoAnswer, detail: String::new() });

        e.sign_in(Engine::Codex, false, now);
        e.on_event(&Event::SignIn { engine: Engine::Codex, step: SignInStep::Browser { url: None } });
        e.disconnected();
        assert_eq!(e.card(Engine::Codex), &Card::Failed { problem: Problem::Lost, detail: String::new() });
        e.keep(Engine::Codex);
        assert_eq!(e.card(Engine::Codex), &Card::Idle);
    }

    #[test]
    fn signing_out_asks_first_and_settles_when_the_daemon_says() {
        let mut e = Engines::default();
        assert!(e.sign_out(Engine::Codex).is_empty(), "never without the question");
        e.ask_sign_out(Engine::Codex);
        e.keep(Engine::Codex);
        assert_eq!(e.card(Engine::Codex), &Card::Idle);
        e.ask_sign_out(Engine::Codex);
        assert_eq!(e.sign_out(Engine::Codex), vec![Command::SignOut { engine: Engine::Codex }]);
        assert_eq!(e.card(Engine::Codex), &Card::SigningOut);
        e.on_event(&Event::Engines { engines: vec![status(Engine::Codex, EngineState::SignedOut)] });
        assert_eq!(e.card(Engine::Codex), &Card::Idle);
        assert!(e.sign_in(Engine::Claude, false, Instant::now()).is_empty(), "Claude Code signs in in a terminal");
    }

    #[test]
    fn the_list_is_asked_once_and_a_silent_daemon_is_said() {
        let mut e = Engines::default();
        let now = Instant::now();
        assert_eq!(e.open(now), vec![Command::Engines]);
        assert!(!e.no_answer(now + Duration::from_secs(1)) && e.no_answer(now + NO_ANSWER));
        e.on_event(&Event::Engines { engines: vec![] });
        assert!(!e.no_answer(now + NO_ANSWER));
        e.attached();
        assert!(e.statuses.is_none());
    }

    #[test]
    fn a_status_reads_in_a_few_words() {
        let mut claude = status(Engine::Claude, EngineState::SignedIn);
        claude.plan = Some("max".into());
        assert_eq!(headline(&claude), (Tone::Ready, "로그인됨 · Max".into()));
        claude.per_token = true;
        assert_eq!(headline(&claude).0, Tone::Ready);
        assert_eq!(headline(&status(Engine::Codex, EngineState::NotInstalled)), (Tone::Needs, "codex CLI 설치 필요".into()));
        assert_eq!(headline(&status(Engine::Codex, EngineState::SignedOut)).0, Tone::Needs);
        assert_eq!(headline(&status(Engine::Codex, EngineState::Unknown)).0, Tone::Unknown);

        let mut codex = status(Engine::Codex, EngineState::SignedIn);
        assert_eq!(members_line(&codex, "acme"), None);
        codex.members = vec![MemberRef { squad: "acme".into(), member: "backend".into() }, MemberRef { squad: "home".into(), member: "pm".into() }];
        assert_eq!(members_line(&codex, "acme").as_deref(), Some("backend, home/pm"));

        let plan = PlanUsage { five_hour: Some(PlanWindow { used_percent: 42, resets_at: 1000 + 70 * 60 }), seven_day: Some(PlanWindow { used_percent: 120, resets_at: 1000 + 3 * 86_400 }) };
        let rows = plan_usage(&plan, 1000);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].0.contains("42%") && rows[0].0.contains("1시간 10분"), "{rows:?}");
        assert_eq!(rows[1].1, 100, "a bar never runs past full");
    }
}
