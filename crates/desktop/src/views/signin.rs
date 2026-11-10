//! The card a turn that lost its engine's sign-in leaves in the
//! conversation, in place of the raw error: what happened, on which machine,
//! and the one thing to do — which equipe found by looking (the daemon's
//! [`Event::EngineAuth`](equipe_core::protocol::Event::EngineAuth)), not
//! guessed from the error's words:
//!
//! - equipe runs outside the Mac's login session (started over SSH), so the
//!   sign-in in its Keychain is out of reach: "Start again in the login session",
//!   no sign-in at all;
//! - the login Keychain is locked: how to unlock it — in the terminal here
//!   when this Mac is the daemon's (its terminal is in the login session);
//!   at that Mac's own screen when it is another (a terminal from here is an
//!   SSH session, which unlocking can't reach equipe from);
//! - signed out: "Sign in again", run by the daemon — the browser opens there,
//!   and from another Mac its page opens here and the code it shows is
//!   pasted back on the card; failing that, the terminal with the command
//!   typed (Enter is the person's);
//! - signed in again: the stopped work carries on by itself (the daemon
//!   does it, as the person would with "Carry on"), and the card says so.
//!
//! The card is live on the latest such error with nothing the person said
//! after it; before that it is the error as it was. Nothing on it approves
//! anything for the person: a sign-in is approved in the browser, a
//! Keychain unlocked with their own password, by them.

use std::time::Instant;

use equipe_core::protocol::{AuthState, Command, Engine};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::convo::Block;
use crate::engines::{self, Card as SignInCard};
use crate::icons::{Icon, icon};
use crate::render::Tones;
use crate::theme::{BODY, BODY_LINE, CONTROL, MONO, Palette, R_LG, R_MD, SMALL, SMALL_LINE};
use crate::ui::{self, Kind};

/// The command that unlocks the login Keychain, typed for the person (the
/// password is theirs to type).
pub const UNLOCK: &str = "security unlock-keychain ~/Library/Keychains/login.keychain-db";
/// Claude Code's own sign-in, typed for the person.
pub const CLAUDE_LOGIN: &str = "claude auth login";
/// What puts the daemon in the login service, at the Mac's own screen.
pub const INSTALL_SERVICE: &str = "equipe daemon install";

/// The row of the conversation's live card: the latest error saying an
/// engine's sign-in was lost, with nothing the person said after it.
pub fn live(blocks: &[Block]) -> Option<(usize, Engine)> {
    let (ix, engine) = blocks.iter().enumerate().rev().find_map(|(ix, b)| match b {
        Block::Error { next, .. } => Some((ix, equipe_core::engine_auth::lost_by(next.as_deref()))),
        _ => None,
    })?;
    let engine = engine?;
    (!blocks[ix + 1..].iter().any(|b| matches!(b, Block::User(_)))).then_some((ix, engine))
}

/// What the card shows, as things stand.
#[derive(Clone)]
pub struct Card {
    pub engine: Engine,
    /// As the daemon last said; `None`: not heard yet.
    pub state: Option<AuthState>,
    pub detail: String,
    /// This conversation's turn waits on it, to carry on by itself.
    pub stopped_here: bool,
    pub sign_in: SignInCard,
    /// The daemon is on another Mac.
    pub remote: bool,
    /// The daemon's machine, by name.
    pub place: String,
    /// The member answered since: it carried on.
    pub went_on: bool,
    pub installing: bool,
    /// The error, in the tool's words.
    pub said: String,
    pub code: Option<Entity<InputState>>,
}

impl Card {
    pub fn of(shell: &Shell, blocks: &[Block], ix: usize, engine: Engine, went_on: bool) -> Self {
        let auth = shell.engines.auth.get(&engine);
        let viewed = shell.model.viewed();
        let said = match &blocks[ix] {
            Block::Error { message, .. } => message.clone(),
            _ => String::new(),
        };
        Self {
            engine,
            state: auth.map(|a| a.state),
            detail: auth.map(|a| a.detail.clone()).unwrap_or_default(),
            stopped_here: auth.is_some_and(|a| viewed.is_some_and(|v| a.stopped.contains(&v))),
            sign_in: shell.engines.card(engine).clone(),
            remote: shell.endpoint.is_remote(),
            place: shell.daemon_view().place,
            went_on,
            installing: matches!(shell.engines.installing(engine), Some(engines::Install::Running)),
            said,
            code: shell.signin_code.clone(),
        }
    }
}

/// How the card reads at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// equipe is at it.
    Working,
    /// Something for the person.
    Needs,
    /// Fixed.
    Done,
}

/// What a button on the card does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Start the daemon again inside the login session.
    Move,
    /// Look again.
    Check,
    /// The terminal here, with this typed.
    Terminal(&'static str),
    SignIn,
    Cancel,
    Install,
    /// The page to open, in this Mac's browser.
    OpenPage,
    /// "Carry on": the person's own carry on.
    CarryOn,
}

/// What the card says and offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    pub tone: Tone,
    pub title: String,
    pub body: String,
    /// A command to show, to run by hand where only the person can — or a
    /// sign-in's one-time code, to enter on its page.
    pub command: Option<String>,
    pub primary: Option<Offer>,
    pub others: Vec<Offer>,
    /// The paste box shows (Claude Code's page, open).
    pub paste: bool,
}

fn engine_name(engine: Engine) -> &'static str {
    engines::platform(engine).name
}

/// The card's words and buttons, by where things stand — no drawing.
pub fn words(card: &Card) -> Words {
    let (engine, place) = (engine_name(card.engine), card.place.as_str());
    let w = |tone, title: String, body: String, primary: Option<Offer>, others: Vec<Offer>| Words { tone, title, body, command: None, primary, others, paste: false };
    let claude = card.engine == Engine::Claude;
    // Signing in, from this card or another screen: how it goes, first.
    match &card.sign_in {
        SignInCard::Starting { .. } | SignInCard::Elsewhere => {
            return w(Tone::Working, t!("signin.signing_in", engine = engine), t!("signin.starting", place = place), None, vec![Offer::Cancel]);
        }
        SignInCard::Browser { .. } => {
            return w(Tone::Working, t!("signin.signing_in", engine = engine), t!("signin.browser_body", place = place), None, vec![Offer::Cancel]);
        }
        SignInCard::Paste { .. } => {
            let body = if card.remote { t!("signin.paste_far") } else { t!("signin.paste_here") };
            return Words { paste: true, ..w(Tone::Working, t!("signin.signing_in", engine = engine), body.to_owned(), None, vec![Offer::OpenPage, Offer::Cancel]) };
        }
        SignInCard::Code { url, code } => {
            let body = t!("signin.code_body", url = url.trim_start_matches("https://"));
            return Words { command: Some(code.clone()), ..w(Tone::Working, t!("signin.signing_in", engine = engine), body, None, vec![Offer::OpenPage, Offer::Cancel]) };
        }
        SignInCard::Failed { problem, detail } => {
            let why = if detail.is_empty() { engines::problem_words(*problem, place) } else { detail.clone() };
            let mut words = w(Tone::Needs, t!("signin.failed_title").to_owned(), why, Some(Offer::SignIn), vec![]);
            if claude && !card.remote {
                words.others.push(Offer::Terminal(CLAUDE_LOGIN));
            } else if claude {
                words.body = format!("{}\n{}", words.body, t!("signin.by_hand_far", place = place));
                words.command = Some(CLAUDE_LOGIN.to_owned());
            }
            return words;
        }
        SignInCard::Idle | SignInCard::ConfirmSignOut | SignInCard::SigningOut => {}
    }
    match card.state {
        None | Some(AuthState::Checking) => w(Tone::Working, t!("signin.checking_title", place = place, engine = engine), t!("signin.checking_body").to_owned(), None, vec![]),
        Some(AuthState::OutsideLogin { movable: true }) => w(Tone::Needs, t!("signin.outside_title", place = place), t!("signin.outside_body", engine = engine), Some(Offer::Move), vec![]),
        Some(AuthState::OutsideLogin { movable: false }) => Words {
            command: Some(INSTALL_SERVICE.to_owned()),
            ..w(Tone::Needs, t!("signin.outside_title", place = place), t!("signin.outside_by_hand", engine = engine, place = place), Some(Offer::Check), vec![])
        },
        Some(AuthState::Moving) => w(Tone::Working, t!("signin.moving_title", place = place), t!("signin.moving_body").to_owned(), None, vec![]),
        // A terminal from here is this Mac's own only when the daemon is on it.
        Some(AuthState::KeychainLocked) if !card.remote => w(Tone::Needs, t!("signin.locked_title", place = place), t!("signin.locked_here", engine = engine), Some(Offer::Terminal(UNLOCK)), vec![Offer::Check]),
        Some(AuthState::KeychainLocked) => w(Tone::Needs, t!("signin.locked_title", place = place), t!("signin.locked_far", engine = engine, place = place), Some(Offer::Check), vec![]),
        Some(AuthState::SignedOut) => w(Tone::Needs, t!("signin.expired_title", place = place, engine = engine), t!("signin.expired_body").to_owned(), Some(Offer::SignIn), vec![]),
        Some(AuthState::NotInstalled) if card.installing => w(Tone::Working, t!("signin.missing_title", place = place, engine = engine), t!("signin.installing").to_owned(), None, vec![]),
        Some(AuthState::NotInstalled) => w(Tone::Needs, t!("signin.missing_title", place = place, engine = engine), t!("signin.missing_body").to_owned(), Some(Offer::Install), vec![]),
        Some(AuthState::SignedIn { resumed }) if resumed > 0 || card.stopped_here || card.went_on => {
            let body = if card.went_on { t!("signin.back_went_on") } else { t!("signin.back_carrying_on") };
            w(Tone::Done, t!("signin.back_title", engine = engine), body.to_owned(), None, vec![])
        }
        // Signed in, but this conversation's turn wasn't kept to carry on (an
        // earlier daemon's, say): one tap, the person's own.
        Some(AuthState::SignedIn { .. }) => w(Tone::Done, t!("signin.back_title", engine = engine), t!("signin.back_ready").to_owned(), Some(Offer::CarryOn), vec![]),
        Some(AuthState::Unknown) => {
            let body = if card.detail.is_empty() { t!("signin.unknown_body").to_owned() } else { card.detail.clone() };
            let mut others = vec![Offer::SignIn];
            if claude && !card.remote {
                others.push(Offer::Terminal(CLAUDE_LOGIN));
            }
            w(Tone::Needs, t!("signin.unknown_title", engine = engine), body, Some(Offer::Check), others)
        }
    }
}

/// The card, drawn.
pub fn card(card: &Card, p: Palette, tones: Tones, key: u64, handle: WeakEntity<Shell>) -> AnyElement {
    let words = words(card);
    let engine = card.engine;
    let (edge, mark): (Rgba, AnyElement) = match words.tone {
        Tone::Working => (p.hair, crate::motion::spinner(14., p.text2)),
        Tone::Needs => (Rgba { a: 0.45, ..tones.warning }, icon(Icon::Warning, 15., tones.warning).into_any_element()),
        Tone::Done => (Rgba { a: 0.40, ..tones.added }, icon(Icon::Check, 15., tones.added).into_any_element()),
    };
    let fill = match words.tone {
        Tone::Needs => Rgba { a: 0.04, ..tones.warning },
        Tone::Done => Rgba { a: 0.04, ..tones.added },
        Tone::Working => p.raised,
    };
    let button = |n: u32, offer: Offer, kind: Kind| -> AnyElement {
        let handle = handle.clone();
        let (label, glyph) = match offer {
            Offer::Move => (t!("signin.move").to_owned(), Some(Icon::Retry)),
            Offer::Check => (t!("signin.check").to_owned(), Some(Icon::Retry)),
            Offer::Terminal(UNLOCK) => (t!("signin.unlock").to_owned(), Some(Icon::Terminal)),
            Offer::Terminal(_) => (t!("signin.terminal").to_owned(), Some(Icon::Terminal)),
            Offer::SignIn if matches!(card.sign_in, SignInCard::Failed { .. }) => (t!("signin.retry").to_owned(), Some(Icon::Retry)),
            Offer::SignIn => (t!("signin.sign_in", engine = engine_name(engine)), None),
            Offer::Cancel => (t!("engines.cancel").to_owned(), None),
            Offer::Install => (t!("signin.install", engine = engine_name(engine)), Some(Icon::ArrowDown)),
            Offer::OpenPage => (t!("engines.open_page").to_owned(), Some(Icon::External)),
            Offer::CarryOn => (t!("chat.resume").to_owned(), Some(Icon::Play)),
        };
        let mut b = ui::button(p, ("signin-offer", key << 4 | u64::from(n)), kind, label);
        if let Some(glyph) = glyph {
            b = b.icon(glyph);
        }
        b.on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.signin_offer(engine, offer, window, cx)).ok();
        })
        .into_any_element()
    };
    let mut buttons: Vec<AnyElement> = Vec::new();
    if let Some(offer) = words.primary {
        buttons.push(button(0, offer, Kind::Primary));
    }
    for (n, offer) in words.others.iter().enumerate() {
        buttons.push(button(n as u32 + 1, *offer, if words.primary.is_none() && n == 0 && *offer != Offer::Cancel { Kind::Secondary } else { Kind::Ghost }));
    }
    let paste = words.paste.then(|| card.code.clone()).flatten().map(|code| {
        let handle = handle.clone();
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(CONTROL + 4.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(R_MD))
                    .border_1()
                    .border_color(p.hair)
                    .bg(p.code_bg)
                    .font_family(MONO)
                    .text_size(px(13.))
                    .child(Input::new(&code).appearance(false).bordered(false)),
            )
            .child(ui::button(p, ("signin-paste", key), Kind::Primary, t!("signin.paste_send")).on_click(move |_, window, cx| {
                handle.update(cx, |shell, cx| shell.signin_paste(engine, window, cx)).ok();
            }))
    });
    let command = words.command.clone().map(|c| {
        div().px_2().py(px(3.)).rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).font_family(MONO).text_size(px(12.5)).text_color(p.text).child(c)
    });
    let said = (!card.said.is_empty() && words.tone != Tone::Done).then(|| {
        div()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(p.muted)
            .child(gpui_kit::base::SelectableText::new(("signin-said", key), t!("signin.said", engine = engine_name(engine), said = card.said.clone())))
    });
    v_flex()
        .w_full()
        .mt_1()
        .mb_1()
        .p(px(14.))
        .gap_2()
        .rounded(px(R_LG))
        .border_1()
        .border_color(edge)
        .bg(fill)
        .child(
            h_flex()
                .gap_2()
                .items_start()
                .child(div().h(px(BODY_LINE)).flex().items_center().flex_none().child(mark))
                .child(div().min_w_0().text_size(px(BODY)).line_height(px(BODY_LINE)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(words.title.clone())),
        )
        .child(
            v_flex()
                .pl(px(22.))
                .gap_2()
                .child(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text2).child(words.body.clone()))
                .children(command.map(|c| h_flex().child(c)))
                .children(paste)
                .when(!buttons.is_empty(), |c| c.child(h_flex().gap_2().flex_wrap().items_center().children(buttons)))
                .children(said),
        )
        .into_any_element()
}

impl Shell {
    /// A button on the sign-in card.
    pub fn signin_offer(&mut self, engine: Engine, offer: Offer, window: &mut Window, cx: &mut Context<Self>) {
        match offer {
            Offer::Move => self.send_command(Command::MoveIntoLogin),
            Offer::Check => {
                for command in self.engines.check(engine, true) {
                    self.send_command(command);
                }
            }
            Offer::Terminal(line) => {
                self.open_terminal(Some(line.to_owned()), window, cx);
                self.composer.flash = Some(t!("term.offered").into());
            }
            Offer::SignIn => self.signin_start(engine, window, cx),
            Offer::Cancel => self.engine_cancel(engine, cx),
            Offer::Install => self.engine_install(engine, cx),
            Offer::OpenPage => {
                let url = match self.engines.card(engine) {
                    SignInCard::Paste { url } | SignInCard::Code { url, .. } => Some(url.clone()),
                    _ => None,
                };
                if let Some(url) = url {
                    cx.open_url(&url);
                }
            }
            Offer::CarryOn => self.resume_stopped(window, cx),
        }
        cx.notify();
    }

    /// "Sign in again": the daemon runs the engine's own sign-in; Claude Code's
    /// page comes with a box here for the code it shows.
    pub fn signin_start(&mut self, engine: Engine, window: &mut Window, cx: &mut Context<Self>) {
        if engine == Engine::Claude && self.signin_code.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("signin.paste_placeholder")));
            let subscription = cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.signin_paste(Engine::Claude, window, cx);
                }
            });
            self.signin_code = Some(input);
            self.signin_sub = Some(subscription);
        }
        for command in self.engines.sign_in_again(engine, self.endpoint.is_remote(), Instant::now()) {
            self.send_command(command);
        }
        self.engines_later(engines::NO_ANSWER, cx);
        cx.notify();
    }

    /// The code in the card's box, to the sign-in's prompt; the box emptied.
    pub fn signin_paste(&mut self, engine: Engine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.signin_code.clone() else { return };
        let code = input.read(cx).value().to_string();
        for command in self.engines.paste(engine, &code) {
            self.send_command(command);
        }
        input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// The viewed conversation ends on a lost sign-in the daemon hasn't said
    /// anything of since this screen attached: it is asked.
    pub fn signin_watch(&mut self) {
        let Some((_, engine)) = self.model.convo().and_then(|c| live(&c.blocks())) else { return };
        if self.engines.auth.contains_key(&engine) {
            return;
        }
        for command in self.engines.check(engine, false) {
            self.send_command(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthState, Block, CLAUDE_LOGIN, Card, Engine, INSTALL_SERVICE, Offer, SignInCard, Tone, UNLOCK, engines, live, words};

    fn card(state: Option<AuthState>) -> Card {
        Card {
            engine: Engine::Claude,
            state,
            detail: String::new(),
            stopped_here: true,
            sign_in: SignInCard::Idle,
            remote: true,
            place: "mac-mini".into(),
            went_on: false,
            installing: false,
            said: "Failed to authenticate: OAuth session expired and could not be refreshed".into(),
            code: None,
        }
    }

    #[test]
    fn the_card_is_the_latest_lost_sign_in_until_the_person_speaks() {
        let error = |next: &str| Block::Error { message: "Failed to authenticate".into(), next: Some(next.into()) };
        let blocks = vec![Block::User("fix it".into()), error(equipe_core::engine_auth::CLAUDE_HINT)];
        assert_eq!(live(&blocks), Some((1, Engine::Claude)));
        let mut said = blocks.clone();
        said.push(Block::User("why?".into()));
        assert_eq!(live(&said), None, "their words lead");
        let other = vec![Block::User("go".into()), Block::Error { message: "boom".into(), next: None }];
        assert_eq!(live(&other), None, "another error is the error as it was");
        let codex = vec![error(equipe_core::engine_auth::CODEX_HINT)];
        assert_eq!(live(&codex), Some((0, Engine::Codex)));
    }

    #[test]
    fn outside_the_login_session_it_moves_equipe_and_never_asks_for_a_sign_in() {
        let words = words(&card(Some(AuthState::OutsideLogin { movable: true })));
        assert_eq!((words.tone, words.primary), (Tone::Needs, Some(Offer::Move)));
        assert!(!words.others.contains(&Offer::SignIn));
        let by_hand = super::words(&card(Some(AuthState::OutsideLogin { movable: false })));
        assert_eq!((by_hand.primary, by_hand.command.as_deref()), (Some(Offer::Check), Some(INSTALL_SERVICE)));
    }

    #[test]
    fn a_locked_keychain_is_unlocked_in_a_terminal_only_on_the_daemons_own_mac() {
        let far = words(&card(Some(AuthState::KeychainLocked)));
        assert_eq!(far.primary, Some(Offer::Check), "a terminal from another Mac is SSH: unlocking there can't reach equipe");
        assert!(!far.others.iter().any(|o| matches!(o, Offer::Terminal(_))));
        let here = words(&Card { remote: false, ..card(Some(AuthState::KeychainLocked)) });
        assert_eq!(here.primary, Some(Offer::Terminal(UNLOCK)));
        assert!(!UNLOCK.contains("-p "), "the password is never typed for the person");
    }

    #[test]
    fn signed_out_it_signs_in_and_from_another_mac_the_code_is_pasted_here() {
        assert_eq!(words(&card(Some(AuthState::SignedOut))).primary, Some(Offer::SignIn));
        let pasting = words(&Card { sign_in: SignInCard::Paste { url: "https://claude.com/cai/oauth/authorize".into() }, ..card(Some(AuthState::SignedOut)) });
        assert!(pasting.paste && pasting.others.contains(&Offer::OpenPage));
        let failed = Card { sign_in: SignInCard::Failed { problem: engines::Problem::Daemon(equipe_core::protocol::SignInFailure::Failed), detail: String::new() }, ..card(Some(AuthState::SignedOut)) };
        assert_eq!(words(&failed).command.as_deref(), Some(CLAUDE_LOGIN), "from another Mac: the command for that Mac's own screen");
        let here = words(&Card { remote: false, ..failed });
        assert!(here.others.contains(&Offer::Terminal(CLAUDE_LOGIN)), "here: the terminal with it typed");
    }

    #[test]
    fn signed_in_again_the_work_carries_on_by_itself_or_with_one_tap() {
        let carried = words(&card(Some(AuthState::SignedIn { resumed: 1 })));
        assert_eq!((carried.tone, carried.primary), (Tone::Done, None));
        let older = words(&Card { stopped_here: false, ..card(Some(AuthState::SignedIn { resumed: 0 })) });
        assert_eq!(older.primary, Some(Offer::CarryOn), "not kept to carry on: the person's one tap");
        let looking = words(&card(None));
        assert_eq!((looking.tone, looking.primary), (Tone::Working, None));
    }
}
