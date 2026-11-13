//! The real view (`--features snapshot`, `EQUIPE_SNAPSHOT_READONLY=1`): the
//! snapshot build attached to the person's own daemon, drawing exactly what
//! their app shows — their squad, their language, their look — for the UI
//! map, and changing nothing.
//!
//! - What goes to the daemon is read-only: a command only when [`reads`]
//!   says it reads (an allowlist; its `match` has no wildcard, so a new
//!   command can't be sent until it is placed), a request only when its
//!   method is one of [`READ_METHODS`]. Everything else is kept back here and
//!   said in the app's log; what went is said too. The hello carries no
//!   language (the daemon's stays) and no browser panel, and the browser's
//!   own stream isn't opened.
//! - Nothing of the person's is written on this Mac: the remembered state,
//!   drafts, the language pick, what the design screen last saw, the inbox
//!   `equipe send-image` fills, uploads, side commands that change things,
//!   the daemon started, restarted or updated, notices and the Dock's badge
//!   ([`blocked`] at each place).
//! - [`Look`]: what the person sees it in, before the first frame — that of
//!   the screen they used last, as it told the daemon (`person_look`, a read:
//!   their app is often on another Mac than the daemon's); only when no
//!   screen ever told one, read (only) from this Mac's own `~/.equipe`.
//!
//! The window draws to the picture, not to the screen (`snapshot.rs`): it is
//! never shown and nothing of the screen is captured.

use std::path::Path;
use std::time::Duration;

use equipe_core::i18n::Lang;
use equipe_core::protocol::look::{PersonLook, SeenLook, Theme};
use equipe_core::protocol::Command;
use equipe_core::protocol::metrics;
use equipe_core::protocol::rpc;

use crate::target::{Appearance, Remembered, Zoom};

/// The variable that turns it on, in a snapshot build.
#[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
pub const ENV: &str = "EQUIPE_SNAPSHOT_READONLY";

/// Whether this run is the real view. Never, outside a snapshot build.
pub fn on() -> bool {
    #[cfg(feature = "snapshot")]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var(ENV).is_ok_and(|v| v == "1"))
    }
    #[cfg(not(feature = "snapshot"))]
    false
}

/// Whether the window is never shown and draws itself, to its pictures
/// alone: the real view, or `EQUIPE_SNAPSHOT_HIDDEN=1` — a snapshot build on
/// the fake daemon, run (and measured) with nothing on the screen and
/// nothing held back. Never, outside a snapshot build.
pub fn hidden() -> bool {
    #[cfg(feature = "snapshot")]
    {
        static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        on() || *HIDDEN.get_or_init(|| std::env::var("EQUIPE_SNAPSHOT_HIDDEN").is_ok_and(|v| v == "1"))
    }
    #[cfg(not(feature = "snapshot"))]
    false
}

/// Whether `command` only reads (or follows what is there): the screen may
/// send it in the real view. Every other one changes something — the board,
/// a conversation, a setting, what the daemon keeps — and is kept back.
pub fn reads(command: &Command) -> bool {
    use Command as C;
    match command {
        // Answered with what is there, to this screen or every screen.
        C::TicketHistory { .. }
        // What a member was handed when it moved to the other engine, and what came before.
        | C::HandOver { .. }
        | C::Logins
        | C::Autopilot
        | C::LoginInsights { .. }
        | C::BrowserProfiles
        | C::Devices
        | C::Engines
        // The providers and their models: listing them changes nothing.
        | C::Providers
        | C::Models { .. }
        | C::Design { .. }
        | C::ProjectDocs { .. }
        | C::SearchDocs { .. }
        | C::DesignVersions { .. }
        | C::DiskScan
        // Where the plans went: a look.
        | C::PlanUse { .. }
        | C::WordsLog { .. }
        // A picture of a member's page now and then: following, as a screen does.
        | C::WatchBrowser { .. } => true,
        C::Start { .. }
        | C::Submit { .. }
        | C::SubmitLater { .. }
        | C::OnPhone { .. }
        | C::UndoHomeChange { .. }
        | C::KeepDocCopy { .. }
        // A side chat runs an engine on the person's account.
        | C::StartSideChat { .. }
        | C::SideChatSubmit { .. }
        | C::CloseSideChat { .. }
        | C::Say { .. }
        // A call starts and steers the squad; snapshots must never do either.
        | C::CallStart { .. }
        | C::CallUtterance { .. }
        | C::CallEnd { .. }
        | C::UndoWords { .. }
        | C::ClearWordsLog { .. }
        | C::SetWordsAll { .. }
        | C::Interrupt { .. }
        | C::Close { .. }
        | C::Accept { .. }
        | C::Merge { .. }
        | C::KeepOnBranch { .. }
        | C::Reopen { .. }
        | C::Resume { .. }
        | C::Requeue { .. }
        | C::Cancel { .. }
        | C::CreateTicket { .. }
        | C::EditTicket { .. }
        | C::EditTickets { .. }
        | C::MoveTicket { .. }
        | C::CommentTicket { .. }
        | C::SetMode { .. }
        | C::Decide { .. }
        | C::AnswerQuestions { .. }
        | C::DismissQuestions { .. }
        | C::ApproveTeamChange { .. }
        | C::DeclineTeamChange { .. }
        | C::ProposeAccept { .. }
        | C::ConfirmAccept { .. }
        | C::DeclineAccept { .. }
        | C::Scale { .. }
        | C::Refresh { .. }
        // What the person looks at is kept for the squad: new work goes there.
        | C::Focus { .. }
        | C::NewConversation { .. }
        | C::MoveConversation { .. }
        | C::RenameConversation { .. }
        | C::CloseConversation { .. }
        | C::ReopenConversation { .. }
        | C::AddProject { .. }
        | C::RemoveProject { .. }
        | C::SetHq { .. }
        | C::DeleteSquad { .. }
        | C::SaveRoutine { .. }
        | C::DeleteRoutine { .. }
        | C::RunRoutine { .. }
        | C::ApproveRoutine { .. }
        | C::DeclineRoutine { .. }
        | C::SetSignalOn { .. }
        | C::SetIdleWake { .. }
        | C::SetResting { .. }
        | C::SaveLogin { .. }
        | C::DeleteLogin { .. }
        | C::ForgetLoginAllowance { .. }
        | C::AllowLogin { .. }
        | C::SetLoginAuto { .. }
        | C::SetLoginAutopilot { .. }
        | C::SetLoginTotp { .. }
        | C::ScanLoginTotp { .. }
        | C::RegisterLoginPasskey { .. }
        | C::ForgetLoginPasskey { .. }
        | C::SaveMailbox { .. }
        | C::SetSmsCodes { .. }
        | C::SetKeepAlive { .. }
        | C::SaveCliLogin { .. }
        | C::DeleteCliLogin { .. }
        | C::SetLoginScope { .. }
        | C::ForgetNeverOffer { .. }
        | C::DismissLoginOffer { .. }
        | C::SaveBrowserProfile { .. }
        | C::DeleteBrowserProfile { .. }
        | C::SaveSecret { .. }
        | C::DismissSecret { .. }
        | C::SetModel { .. }
        | C::ShowBrowser { .. }
        | C::Desktop { .. }
        | C::DesktopCheck { .. }
        | C::DesktopAuto { .. }
        | C::DesktopLocked { .. }
        | C::Suggestions { .. }
        | C::Unqueue { .. }
        | C::SendNow { .. }
        | C::StopCommand { .. }
        | C::StopSubagent { .. }
        | C::SetEffort { .. }
        | C::BrowserNav { .. }
        | C::BrowserNewTab { .. }
        | C::BrowserCloseTab { .. }
        | C::BrowserControl { .. }
        | C::BrowserDialog { .. }
        | C::BrowserPopOut { .. }
        | C::BrowserSignIn { .. }
        // The language equipe speaks to the person in, kept.
        | C::Language { .. }
        | C::Disallow { .. }
        // A code opened at the relay.
        | C::PairStart
        | C::PairAnswer { .. }
        | C::PairCancel
        | C::Unpair { .. }
        | C::DiskClean { .. }
        | C::DiskRun { .. }
        | C::SignIn { .. }
        | C::SignInCancel { .. }
        // Looking again may carry stopped turns on: the person's own, not a picture's.
        | C::SignInCode { .. }
        | C::CheckEngine { .. }
        | C::MoveIntoLogin
        | C::InstallEngine { .. }
        | C::SetNebiusKey { .. }
        | C::AddProvider { .. }
        | C::TestProvider { .. }
        | C::RemoveProvider { .. }
        | C::SignOut { .. }
        | C::SetEngineSwitching { .. }
        | C::SetPmAccepts { .. }
        | C::SetSubagentsAtOnce { .. }
        | C::SetParallel { .. }
        | C::SetModelChoice { .. }
        | C::SetTicketTier { .. }
        | C::SetDesignReviewer { .. }
        | C::SetRoleModel { .. }
        | C::ApplyEngineProposal { .. }
        | C::DismissEngineProposal { .. }
        | C::UndoEngineProposal { .. }
        | C::ShowEngineSetup { .. }
        | C::MoveEngine { .. }
        | C::RestoreDesign { .. }
        | C::Taste { .. } => false,
    }
}

/// The requests the real view may make: the handshake, and answers read
/// from what is there. An upload, a telemetry answer, a metrics key or
/// refresh, a disk cleaned, a browser window opened — and any method not
/// named here — are kept back.
pub const READ_METHODS: &[&str] = &[
    rpc::SQUADS,
    rpc::HELLO,
    rpc::VERSION,
    rpc::BUSY,
    rpc::DISK,
    rpc::SQUAD_FOOTPRINT,
    rpc::USAGE,
    rpc::READ_UPLOAD,
    rpc::READ_IMAGE,
    rpc::LIST_IMAGES,
    rpc::DESIGN_STUDIO,
    rpc::TELEMETRY,
    rpc::PERSON_LOOK,
    metrics::SOURCES,
    metrics::QUERY,
    metrics::SUMMARY,
];

pub fn reads_method(method: &str) -> bool {
    READ_METHODS.contains(&method)
}


/// The command's name on the wire (`submit`), for the log.
fn name(command: &Command) -> String {
    serde_json::to_value(command).ok().and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned)).unwrap_or_default()
}

/// Whether `command` may go to the daemon now: always, but in the real
/// view only one that reads — said either way in the log.
pub fn lets_command(command: &Command) -> bool {
    if !on() {
        return true;
    }
    let ok = reads(command);
    eprintln!("equipe readonly: {} command {}", if ok { "sent" } else { "kept back" }, name(command));
    ok
}

/// The same of a request, by its method.
pub fn lets_method(method: &str) -> bool {
    if !on() {
        return true;
    }
    let ok = reads_method(method);
    eprintln!("equipe readonly: {} request {method}", if ok { "sent" } else { "kept back" });
    ok
}

/// In the real view: `what` is left as it is (said in the log), and the
/// caller does nothing. Outside it, `false`.
pub fn blocked(what: &str) -> bool {
    if on() {
        eprintln!("equipe readonly: left {what} as it is");
        true
    } else {
        false
    }
}

/// What the person sees the app in: their language, light or dark, and
/// how they left its window and sidebar — as the screen they used last told
/// the daemon, else read (only) from their own `~/.equipe`, as their app
/// reads it when it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub lang: Lang,
    pub appearance: Appearance,
    /// The window, in points: as left, else as the app opens it.
    pub window: (f32, f32),
    pub sidebar_width: Option<f32>,
    pub sidebar_hidden: bool,
    /// The sidebar's projects folded to their header.
    pub folded: Vec<String>,
    pub team_open: bool,
    pub usage_folded: bool,
    pub design_talk: Option<bool>,
    pub zoom: u16,
    /// Whose screen it is; none: this Mac's own state.
    pub from: Option<LookFrom>,
}

/// The screen a [`Look`] is of: its machine and its app's build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookFrom {
    pub machine: String,
    pub version: String,
    pub commit: Option<String>,
    /// When the daemon last heard of it, and when the person last used it (unix seconds).
    pub seen: u64,
    pub active: Option<u64>,
}

/// The window's size when the app opens and none was kept.
pub const WINDOW: (f32, f32) = (1200., 760.);

static LOOK: std::sync::OnceLock<Option<Look>> = std::sync::OnceLock::new();

/// At launch: in the real view, the person's look, asked once (and said in
/// the log); `None` otherwise.
pub fn begin() -> Option<&'static Look> {
    LOOK.get_or_init(|| {
        on().then(|| {
            let home = equipe_providers::credentials::equipe_home();
            let look = Look::choose(person_look(&home.join("equipe.sock")), || Look::read(&home, &mac_languages()));
            match &look.from {
                Some(from) => eprintln!("equipe readonly: the person's look, from their screen on {} ({}): {}", from.machine, from.version, look.steps()),
                None => eprintln!("equipe readonly: the person's look, from this Mac's own state (no screen ever told one): {}", look.steps()),
            }
            look
        })
    })
    .as_ref()
}

/// How long the daemon may take to say the person's look.
const ASK_WAIT: Duration = Duration::from_secs(5);

/// The look of the screen the person used last, asked of the daemon at
/// `socket` (`person_look`, which reads): `None` when no screen ever told one
/// — or the daemon can't say (one older than this).
#[cfg(unix)]
pub fn person_look(socket: &Path) -> Option<SeenLook> {
    use std::io::{BufRead as _, Write as _};
    if !lets_method(rpc::PERSON_LOOK) {
        return None;
    }
    let stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(ASK_WAIT)).ok()?;
    stream.set_write_timeout(Some(ASK_WAIT)).ok()?;
    (&stream).write_all(rpc::Message::request(1, rpc::PERSON_LOOK, serde_json::json!({})).to_line().as_bytes()).ok()?;
    for line in std::io::BufReader::new(&stream).lines() {
        let message = rpc::Message::from_line(&line.ok()?).ok()?;
        if message.id == Some(1) {
            return message.result_as::<PersonLook>()?.look;
        }
    }
    None
}

#[cfg(windows)]
pub fn person_look(_socket: &Path) -> Option<SeenLook> {
    None
}

/// The person's look, once [`begin`] read it.
pub fn look() -> Option<&'static Look> {
    LOOK.get().and_then(Option::as_ref)
}

impl Look {
    /// From `home`; `mac`: the languages the Mac prefers for the app, most
    /// preferred first — what decides when the person picked none.
    pub fn read(home: &Path, mac: &[String]) -> Self {
        let remembered = Remembered::load(home);
        // The shared pick; else the app's own from before it (which the app
        // makes the shared one as it starts); else the Mac's.
        let lang = equipe_core::i18n::saved(home)
            .or_else(|| remembered.lang.as_deref().and_then(Lang::parse))
            .unwrap_or_else(|| mac.iter().find_map(|tag| Lang::parse(tag)).unwrap_or(Lang::En));
        Self {
            lang,
            appearance: remembered.appearance,
            window: remembered.window.unwrap_or(WINDOW),
            sidebar_width: remembered.sidebar_width,
            sidebar_hidden: false,
            folded: remembered.folded.into_iter().collect(),
            team_open: remembered.team_open,
            usage_folded: remembered.usage_folded,
            design_talk: remembered.design_talk,
            zoom: remembered.zoom.0,
            from: None,
        }
    }

    /// The look a screen told: as it showed the app, to the window's size.
    pub fn of_screen(seen: &SeenLook) -> Self {
        let look = &seen.look;
        let window = if look.width > 0 && look.height > 0 { (look.width as f32, look.height as f32) } else { WINDOW };
        Self {
            lang: Lang::parse(&look.lang).unwrap_or(Lang::En),
            appearance: match look.theme {
                Theme::Light => Appearance::Light,
                Theme::Dark => Appearance::Dark,
            },
            window,
            sidebar_width: look.sidebar_width.map(|w| w as f32),
            sidebar_hidden: look.sidebar_hidden,
            folded: look.folded.clone(),
            team_open: look.team_open,
            usage_folded: look.usage_folded,
            design_talk: look.design_talk,
            zoom: Zoom::nearest(look.zoom).0,
            from: Some(LookFrom { machine: look.machine.clone(), version: look.version.clone(), commit: look.commit.clone(), seen: seen.seen, active: seen.active }),
        }
    }

    /// The screen's look when one ever told the daemon; else `local`, this
    /// Mac's own.
    pub fn choose(told: Option<SeenLook>, local: impl FnOnce() -> Self) -> Self {
        match told {
            Some(seen) => Self::of_screen(&seen),
            None => local(),
        }
    }

    /// The app's remembered state with this look laid over it, as it starts:
    /// the sidebar, the folds, the zoom — what the steps don't bring.
    pub fn lay_over(&self, remembered: &mut Remembered) {
        remembered.appearance = self.appearance;
        remembered.window = Some(self.window);
        remembered.sidebar_width = self.sidebar_width;
        remembered.folded = self.folded.iter().cloned().collect();
        remembered.team_open = self.team_open;
        remembered.usage_folded = self.usage_folded;
        remembered.design_talk = self.design_talk;
        remembered.zoom = Zoom(self.zoom);
    }

    /// For the note beside a picture: whose look it was shot in.
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn note(&self) -> serde_json::Value {
        match &self.from {
            Some(from) => serde_json::json!({ "machine": from.machine, "version": from.version, "commit": from.commit, "seen": from.seen, "active": from.active }),
            None => serde_json::json!({ "machine": crate::screen_look::machine(), "local": true }),
        }
    }

    /// The steps (`snapshot.rs`'s) that bring the window to it, before the
    /// script's own — which may say otherwise. The rest of it (the sidebar,
    /// the folds, the zoom) is laid over what the app starts with
    /// ([`Look::lay_over`]).
    pub fn steps(&self) -> String {
        let mut steps = vec![format!("lang:{}", self.lang.code()), format!("size:{}x{}", self.window.0.round(), self.window.1.round())];
        match self.appearance {
            Appearance::Light => steps.push("theme:light".into()),
            Appearance::Dark => steps.push("theme:dark".into()),
            Appearance::System => {}
        }
        if self.sidebar_hidden {
            steps.push("sidebar".into());
        }
        steps.join(";")
    }
}

/// The languages the Mac prefers for the installed app: its own pick in
/// System Settings (`AppleLanguages` of `im.equipe.desktop`), else the Mac's.
pub fn mac_languages() -> Vec<String> {
    let read = |domain: &str| -> Vec<String> {
        let out = std::process::Command::new("/usr/bin/defaults").args(["read", domain, "AppleLanguages"]).stderr(std::process::Stdio::null()).output();
        let Ok(out) = out else { return Vec::new() };
        if !out.status.success() {
            return Vec::new();
        }
        parse_languages(&String::from_utf8_lossy(&out.stdout))
    };
    let own = read("im.equipe.desktop");
    if own.is_empty() { read("-g") } else { own }
}

/// `defaults read`'s list, `("ko-KR", en)`: its words in order.
pub fn parse_languages(text: &str) -> Vec<String> {
    text.trim().trim_start_matches('(').trim_end_matches(')').split(',').map(|w| w.trim().trim_matches('"').to_owned()).filter(|w| !w.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::design::TasteChange;
    use equipe_core::protocol::{accept, decide, engines, routines, secrets, vault};
    use equipe_core::protocol::{Access, BrowserNavAction, BrowserProfile, Decision, Engine, Mode, ScreenBounds, SessionSpec, TicketDraft, TicketEdit};

    /// One of each command there is.
    fn every_command() -> Vec<Command> {
        let s = || "acme".to_owned();
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: "/".into(), role: None, squad: None };
        let routine = routines::Routine { id: 0, title: "t".into(), prompt: "p".into(), when: routines::Schedule::Daily { hour: 9, minute: 0 }, on: true, last_run: None, next_run: None };
        let login = vault::LoginDraft { id: None, site: "x".into(), username: "u".into(), password: None, note: None, auto: None, idp: None };
        vec![
            Command::Start { session: 1, spec },
            Command::Submit { session: 1, text: "x".into(), about: None },
            Command::SubmitLater { session: 1, text: "x".into(), about: None },
            Command::Interrupt { session: 1 },
            Command::Close { session: 1 },
            Command::StartSideChat { session: 1, side: 2, passage: "x".into() },
            Command::SideChatSubmit { session: 1, side: 2, passage: "x".into(), text: "x".into() },
            Command::CloseSideChat { side: 2 },
            Command::Accept { squad: s(), ticket: 1 },
            Command::Merge { squad: s(), ticket: 1, push: false },
            Command::KeepOnBranch { squad: s(), ticket: 1 },
            Command::Reopen { squad: s(), ticket: 1, note: "n".into() },
            Command::Resume { squad: s(), ticket: 1, note: "n".into() },
            Command::Requeue { squad: s(), ticket: 1, operation: "o".into(), how: equipe_core::protocol::Recovery::Resume },
            Command::Cancel { squad: s(), ticket: 1 },
            Command::CreateTicket { squad: s(), draft: TicketDraft::default() },
            Command::EditTicket { squad: s(), ticket: 1, changes: TicketEdit::default() },
            Command::EditTickets { squad: s(), tickets: vec![1], changes: TicketEdit::default() },
            Command::MoveTicket { squad: s(), ticket: 1, status: None, before: None },
            Command::CommentTicket { squad: s(), ticket: 1, text: "x".into() },
            Command::TicketHistory { squad: s(), ticket: 1 },
            Command::HandOver { squad: s(), session: 1 },
            Command::SetMode { squad: s(), member: None, mode: Some(Mode::default()) },
            Command::Decide { squad: s(), approval: 1, decision: Decision::Allow, words: None, said: None },
            Command::AnswerQuestions { squad: s(), id: 1, answers: vec![decide::Answer::default()], about: None },
            Command::DismissQuestions { squad: s(), id: 1 },
            Command::ApproveTeamChange { squad: s(), id: 1 },
            Command::DeclineTeamChange { squad: s(), id: 1 },
            Command::ProposeAccept { squad: s(), session: 1, verdict: accept::Verdict::Accept, tickets: vec![1], note: String::new() },
            Command::ConfirmAccept { squad: s(), id: 1, tickets: vec![accept::Confirmed::only(1)], said: None },
            Command::DeclineAccept { squad: s(), id: 1 },
            Command::Scale { squad: s(), extra: 1 },
            Command::Refresh { session: 1, summary: true },
            Command::Focus { squad: s(), project: None },
            Command::NewConversation { squad: s(), session: 1, project: None },
            Command::MoveConversation { session: 1, project: None },
            Command::RenameConversation { squad: s(), session: 1, title: "t".into() },
            Command::CloseConversation { session: 1 },
            Command::ReopenConversation { squad: s(), session: 1 },
            Command::AddProject { squad: s(), path: "/p".into() },
            Command::RemoveProject { squad: s(), path: "/p".into() },
            Command::SetHq { squad: s(), path: None },
            Command::DeleteSquad { squad: s(), purge: false },
            Command::SaveRoutine { squad: s(), routine },
            Command::DeleteRoutine { squad: s(), id: 1 },
            Command::RunRoutine { squad: s(), id: 1 },
            Command::ApproveRoutine { squad: s(), id: 1 },
            Command::DeclineRoutine { squad: s(), id: 1 },
            Command::SetSignalOn { squad: s(), project: "p".into(), name: "n".into(), on: false },
            Command::SetIdleWake { squad: s(), hours: 0 },
            Command::SetResting { squad: s(), member: "qa".into(), resting: true, stop: false },
            Command::SaveLogin { draft: login },
            Command::DeleteLogin { id: 1 },
            Command::ForgetLoginAllowance { id: 1, who: "acme/pm".into() },
            Command::AllowLogin { id: 1, who: "acme".into() },
            Command::SetLoginAuto { id: 1, auto: true },
            Command::SetLoginAutopilot { id: 1, autopilot: Default::default() },
            Command::SetLoginTotp { id: 1, secret: None },
            Command::ScanLoginTotp { id: 1 },
            Command::RegisterLoginPasskey { id: 1 },
            Command::ForgetLoginPasskey { id: 1 },
            Command::SaveMailbox { mailbox: None },
            Command::SetSmsCodes { on: true },
            Command::SetKeepAlive { hours: 6 },
            Command::SaveCliLogin { draft: equipe_core::protocol::vault::CliLoginDraft { id: None, kind: equipe_core::protocol::vault::CliKind::Env, name: "NPM_TOKEN".into(), host: String::new(), public: String::new(), squad: None, secret: None } },
            Command::DeleteCliLogin { id: 1 },
            Command::SetLoginScope { id: 1, squads: vec!["acme".into()] },
            Command::ForgetNeverOffer { site: "github.com".into() },
            Command::DismissLoginOffer { id: 1, never: true },
            Command::Logins,
            Command::Autopilot,
            Command::LoginInsights { days: 7 },
            Command::SaveBrowserProfile { profile: BrowserProfile { name: "b".into(), label: String::new() } },
            Command::DeleteBrowserProfile { name: "b".into() },
            Command::BrowserProfiles,
            Command::SaveSecret { squad: s(), id: 1, values: vec![secrets::SecretValue("v".into())] },
            Command::DismissSecret { squad: s(), id: 1 },
            Command::WatchBrowser { squad: s(), session: None },
            Command::SetModel { session: 1, model: "m".into() },
            Command::ShowBrowser { squad: s(), member: None, show: true },
            Command::Desktop { open: false },
            Command::DesktopCheck { guide: true },
            Command::DesktopAuto { action: equipe_core::protocol::DesktopAutoAction::Retry },
            Command::DesktopLocked { on: true },
            Command::Suggestions { squad: s(), on: false },
            Command::Unqueue { session: 1, text: "x".into() },
            Command::SendNow { session: 1, texts: vec![] },
            Command::StopCommand { session: 1 },
            Command::StopSubagent { session: 1, id: "h1".into() },
            Command::SetEffort { session: 1, effort: None },
            Command::BrowserNav { page: 1, action: BrowserNavAction::Reload, url: None },
            Command::BrowserNewTab { url: None, profile: None },
            Command::BrowserCloseTab { page: 1 },
            Command::BrowserControl { lane: "pm".into(), take: true },
            Command::BrowserDialog { page: 1, accept: true, text: None },
            Command::BrowserPopOut { page: 1, bounds: Some(ScreenBounds { left: 0, top: 0, width: 1, height: 1 }) },
            Command::BrowserSignIn { url: "github.com".into(), window: false },
            Command::Language { lang: "en".into() },
            Command::Disallow { squad: s(), rule: "r".into() },
            Command::PairStart,
            Command::PairAnswer { approve: true },
            Command::PairCancel,
            Command::Devices,
            Command::Unpair { device: "d".into() },
            Command::DiskScan,
            Command::PlanUse { engine: Engine::Claude, span: equipe_core::protocol::plan_use::Span::Week },
            Command::Say { squad: s(), session: 1, text: "x".into(), via: Default::default(), all: false, later: false, pasted: false, about: None, device: None },
            Command::CallStart { squad: s() },
            Command::CallUtterance { squad: s(), call: "c".into(), text: "x".into(), via: Default::default() },
            Command::CallEnd { squad: s(), call: "c".into() },
            Command::UndoWords { squad: s(), id: 1 },
            Command::WordsLog { squad: s() },
            Command::ClearWordsLog { squad: s() },
            Command::SetWordsAll { squad: s(), on: true },
            Command::DiskClean { ids: vec![] },
            Command::DiskRun { id: "i".into() },
            Command::Engines,
            Command::SignIn { engine: Engine::Codex, device: false },
            Command::SignInCancel { engine: Engine::Codex },
            Command::SignInCode { engine: Engine::Claude, code: "c".into() },
            Command::CheckEngine { engine: Engine::Claude },
            Command::MoveIntoLogin,
            Command::InstallEngine { engine: Engine::Claude },
            Command::SetNebiusKey { key: None },
            Command::Providers,
            Command::AddProvider { preset: "ollama".into(), base_url: None, key: None },
            Command::TestProvider { id: "ollama".into() },
            Command::RemoveProvider { id: "ollama".into() },
            Command::SignOut { engine: Engine::Codex },
            Command::SetEngineSwitching { squad: s(), mode: engines::Switching::default() },
            Command::SetPmAccepts { squad: s(), on: false },
            Command::SetSubagentsAtOnce { squad: s(), at_once: Some(4) },
            Command::SetParallel { squad: s(), parallel: equipe_core::protocol::lanes::Parallel::max(6, 16) },
            Command::SetModelChoice { squad: s(), choice: equipe_core::protocol::tiers::ModelChoice::Off },
            Command::SetTicketTier { squad: s(), ticket: 1, tier: Some(equipe_core::protocol::tiers::ModelTier::Max), pinned: true },
            Command::SetDesignReviewer { squad: s(), reviewer: engines::Reviewer::Same },
            Command::Models { engine: Engine::Codex },
            Command::SetRoleModel { squad: s(), default: equipe_core::protocol::RoleModel { role: "qa".into(), engine: Engine::Claude, model: None, effort: None } },
            Command::ApplyEngineProposal { squad: s(), id: 1, now: false },
            Command::DismissEngineProposal { squad: s(), id: 1 },
            Command::UndoEngineProposal { squad: s(), id: 1 },
            Command::UndoHomeChange { squad: s(), session: 1, change: "abc".into() },
            Command::OnPhone { command: Box::new(Command::Interrupt { session: 1 }) },
            Command::ShowEngineSetup { squad: s() },
            Command::MoveEngine { squad: s(), members: vec![], engine: Engine::Claude, model: None, effort: None, now: false },
            Command::Design { squad: s() },
            Command::ProjectDocs { squad: s() },
            Command::SearchDocs { squad: s(), query: "checkout".into() },
            Command::KeepDocCopy { squad: s(), session: 1, keep: "/h/docs/web/plans/2026-11-04-x.md".into() },
            Command::DesignVersions { squad: s(), project: None },
            Command::RestoreDesign { squad: s(), project: None, commit: "c".into() },
            Command::Taste { squad: s(), project: None, change: TasteChange::Add { text: "t".into() } },
        ]
    }

    /// Every variant `Command` has, by its name on the wire, read from the
    /// enum's own source: a new one fails here until it is sampled above.
    fn every_variant() -> Vec<String> {
        let source = include_str!("../../protocol/src/lib.rs");
        let body = source.split("pub enum Command {").nth(1).expect("the enum").split("\n}\n").next().expect("its end");
        let mut out = Vec::new();
        for line in body.lines() {
            let Some(rest) = line.strip_prefix("    ") else { continue };
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_uppercase()) || !rest[name.len()..].starts_with([' ', ',', '{', '(']) {
                continue;
            }
            let mut snake = String::new();
            for (n, c) in name.chars().enumerate() {
                if c.is_ascii_uppercase() && n > 0 {
                    snake.push('_');
                }
                snake.push(c.to_ascii_lowercase());
            }
            out.push(snake);
        }
        out
    }

    #[test]
    fn every_command_is_placed_and_only_reads_and_subscriptions_go() {
        let commands = every_command();
        let mut sampled: Vec<String> = commands.iter().map(name).collect();
        let mut variants = every_variant();
        assert!(variants.len() > 80, "the enum was read: {variants:?}");
        sampled.sort();
        variants.sort();
        assert_eq!(sampled, variants, "each command once: a new one is sampled here, and placed in `reads`");
        let going: Vec<String> = commands.iter().filter(|c| reads(c)).map(name).collect();
        assert_eq!(going, ["ticket_history", "hand_over", "logins", "autopilot", "login_insights", "browser_profiles", "watch_browser", "devices", "disk_scan", "plan_use", "words_log", "engines", "providers", "models", "design", "project_docs", "search_docs", "design_versions"]);
        // What a screen does as the person works is never among them.
        for kept in ["submit", "say", "undo_words", "decide", "accept", "merge", "create_ticket", "move_ticket", "answer_questions", "confirm_accept", "focus", "language", "start", "start_side_chat", "side_chat_submit", "taste", "disk_clean", "add_provider", "remove_provider", "test_provider", "keep_doc_copy"] {
            assert!(!going.iter().any(|g| g == kept), "{kept} changes something");
        }
        // Outside the real view, everything goes, as ever.
        assert!(!on() && commands.iter().all(lets_command));
    }

    #[test]
    fn only_requests_that_read_go() {
        for method in [rpc::HELLO, rpc::SQUADS, rpc::USAGE, rpc::READ_IMAGE, rpc::LIST_IMAGES, rpc::DESIGN_STUDIO, rpc::READ_UPLOAD, rpc::PERSON_LOOK, metrics::QUERY, metrics::SUMMARY, metrics::SOURCES] {
            assert!(reads_method(method), "{method}");
        }
        // The real view tells no look of its own: it isn't the person's screen.
        for method in [rpc::SCREEN_LOOK, rpc::UPLOAD, rpc::TELEMETRY_SET, rpc::DISK_CLEAN, rpc::BROWSER_WINDOW, rpc::BROWSER_SIGN_IN, rpc::BROWSER_STREAM, rpc::SHUTDOWN, rpc::TOOL, rpc::COMMAND, rpc::DESKTOP, metrics::CONNECT, metrics::DISCONNECT, metrics::REFRESH, "metrics.anything_new"] {
            assert!(!reads_method(method), "{method}");
        }
        assert!(!blocked("nothing") && lets_method(rpc::UPLOAD), "outside the real view nothing is held back");
    }

    #[test]
    fn the_look_is_read_from_the_persons_remembered_state_and_brings_the_window_to_it() {
        let home = std::env::temp_dir().join(format!("equipe-look-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // Nothing kept: the app as it opens, in the Mac's first language equipe speaks.
        let look = Look::read(&home, &["pt-BR".into(), "ko-KR".into()]);
        assert_eq!((look.lang, look.appearance, look.window, look.sidebar_width, look.zoom), (Lang::Ko, Appearance::System, WINDOW, None, 100));
        assert_eq!(look.steps(), "lang:ko;size:1200x760");
        // As the person left it.
        std::fs::write(Remembered::path(&home), "sidebar-width 312\nfold\tweb\nfold\t\nappearance dark\nwindow-size 1440x900\nzoom 110\nbrowser-width 559\n").unwrap();
        equipe_core::i18n::save(&home, Some(Lang::En)).unwrap();
        let before = std::fs::read(Remembered::path(&home)).unwrap();
        let look = Look::read(&home, &["ko-KR".into()]);
        let left = Look {
            lang: Lang::En,
            appearance: Appearance::Dark,
            window: (1440., 900.),
            sidebar_width: Some(312.),
            sidebar_hidden: false,
            folded: vec![String::new(), "web".into()],
            team_open: false,
            usage_folded: false,
            design_talk: None,
            zoom: 110,
            from: None,
        };
        assert_eq!(look, left);
        assert_eq!(look.steps(), "lang:en;size:1440x900;theme:dark", "a step of the script after it still says otherwise");
        assert_eq!(std::fs::read(Remembered::path(&home)).unwrap(), before, "read, never written");
        std::fs::remove_dir_all(home).unwrap();
        assert_eq!(parse_languages("(\n    \"ko-KR\",\n    en\n)\n"), ["ko-KR", "en"]);
        assert!(parse_languages("").is_empty());
    }

    fn told(machine: &str, lang: &str, sidebar: Option<u32>) -> SeenLook {
        use equipe_core::protocol::look::ScreenLook;
        let look = ScreenLook {
            machine: machine.into(),
            app: "equipe-desktop".into(),
            version: "0.103.25".into(),
            commit: Some("5a1288f".into()),
            lang: lang.into(),
            theme: Theme::Light,
            width: 1300,
            height: 820,
            sidebar_width: sidebar,
            sidebar_hidden: false,
            folded: vec!["web".into()],
            team_open: true,
            usage_folded: true,
            design_talk: Some(false),
            zoom: 115,
            idle: None,
        };
        SeenLook { look, seen: 200, active: Some(190) }
    }

    #[test]
    fn the_look_of_the_persons_own_screen_wins_and_this_macs_only_when_none_ever_said() {
        // This Mac's own state: Korean, dark, wide — the daemon's Mac, which nobody looks at.
        let home = std::env::temp_dir().join(format!("equipe-look-told-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(Remembered::path(&home), "sidebar-width 312\nappearance dark\nwindow-size 1440x900\n").unwrap();
        equipe_core::i18n::save(&home, Some(Lang::Ko)).unwrap();
        let local = || Look::read(&home, &["ko-KR".into()]);
        // A screen told one: it wins, whatever this Mac says.
        let look = Look::choose(Some(told("MacBook", "en", Some(180))), local);
        assert_eq!((look.lang, look.appearance, look.window, look.sidebar_width, look.zoom), (Lang::En, Appearance::Light, (1300., 820.), Some(180.), 110));
        assert_eq!(look.from.as_ref().map(|f| (f.machine.as_str(), f.version.as_str(), f.active)), Some(("MacBook", "0.103.25", Some(190))));
        assert_eq!(look.steps(), "lang:en;size:1300x820;theme:light");
        assert_eq!(look.note()["machine"], "MacBook");
        assert_eq!(look.note()["version"], "0.103.25");
        // What the steps don't bring is laid over what the app starts with.
        let mut remembered = Remembered::load(&home);
        look.lay_over(&mut remembered);
        assert_eq!((remembered.sidebar_width, remembered.appearance, remembered.zoom, remembered.team_open, remembered.usage_folded, remembered.design_talk), (Some(180.), Appearance::Light, Zoom(110), true, true, Some(false)));
        assert_eq!(remembered.folded.iter().collect::<Vec<_>>(), ["web"]);
        // The sidebar hidden there is hidden here; an unknown language reads as English.
        let mut hidden = told("MacBook", "xx", None);
        hidden.look.sidebar_hidden = true;
        let look = Look::of_screen(&hidden);
        assert_eq!((look.lang, look.sidebar_width), (Lang::En, None));
        assert!(look.steps().ends_with(";sidebar"), "{}", look.steps());
        // None ever said: this Mac's own, read only.
        let look = Look::choose(None, local);
        assert_eq!((look.lang, look.appearance, look.sidebar_width, look.from.clone()), (Lang::Ko, Appearance::Dark, Some(312.), None));
        assert_eq!(look.note()["local"], true);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn the_daemon_is_asked_for_the_persons_look_and_a_daemon_that_cant_say_is_none() {
        use std::io::{BufRead as _, Write as _};
        let socket = std::path::PathBuf::from(format!("/tmp/eqro-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let answers = [Some(PersonLook { look: Some(told("MacBook", "en", Some(180))) }), Some(PersonLook::default()), None];
        let serving = std::thread::spawn(move || {
            let mut asked = Vec::new();
            for answer in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
                let ask = rpc::Message::from_line(&line).unwrap();
                asked.push(ask.method.clone().unwrap_or_default());
                let id = ask.id.unwrap();
                let reply = match answer {
                    Some(answer) => rpc::Message::response(id, answer),
                    None => rpc::Message::failure(id, -32601, "unknown method"),
                };
                (&stream).write_all(reply.to_line().as_bytes()).unwrap();
            }
            asked
        });
        assert_eq!(person_look(&socket).map(|s| s.look.machine), Some("MacBook".into()));
        assert_eq!(person_look(&socket), None, "no screen ever said");
        assert_eq!(person_look(&socket), None, "an older daemon");
        assert_eq!(serving.join().unwrap(), [rpc::PERSON_LOOK; 3]);
        let _ = std::fs::remove_file(&socket);
        assert_eq!(person_look(&socket), None, "no daemon");
    }
}
