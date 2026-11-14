//! `cli_sign_in` on the daemon: the tool's own cross-device sign-in run
//! here, in a pseudo-terminal, while its page and code are on a card on every
//! screen (`crate::cli_signin`). The cards live as long as the process: they
//! aren't journaled (a restart ends the process they stand for). What the
//! person pastes goes to the process and is dropped — it is not in a card,
//! an event, a journal or a log. When it ends signed in, the member is woken
//! with as whom.
//!
//! An outside MCP server's sign-in (`crate::squad_mcp`) is a card of the same
//! kind, started by equipe: when it ends signed in, the server is checked
//! again and its members have it from their next turn; nobody is woken.

use crate::cli_signin::{self as cs, Kind, Outcome};
use crate::protocol::cli_signin::{CliSignIn, PastedCode, Phase};

use super::*;

/// How long a signed-in card stays, to be seen ticked.
const TICKED_FOR: Duration = Duration::from_secs(20);

/// A sign-in running (or waiting to be run again) for a card.
pub(super) struct Live {
    pub card: CliSignIn,
    kind: Kind,
    plan: cs::Plan,
    /// Which run of the process the notes are of: a restart makes a new one.
    run: u64,
    /// Where pasted codes go, while the process waits.
    codes: Option<UnboundedSender<PastedCode>>,
    task: Option<JoinHandle<()>>,
    /// The device sign-in approval equipe took over for this run, where the
    /// person let it approve that tool's sign-ins (login autopilot).
    approving: Option<u32>,
    /// The outside MCP server it signs in to, for an MCP sign-in.
    pub server: Option<String>,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// What the process, off the loop, tells the runtime.
#[derive(Debug)]
pub(super) enum Note {
    /// Its page or its code became known.
    Seen { squad: String, id: u64, run: u64, url: Option<String>, code: Option<String> },
    /// It ended; `who` when it signed in and the tool said as whom.
    Ended { squad: String, id: u64, run: u64, outcome: Outcome, who: Option<String> },
    /// A signed-in card has been seen long enough.
    Sweep { squad: String, id: u64 },
}

impl Runtime {
    fn cli_signins(&mut self, squad: &str) -> &mut Vec<Live> {
        self.cli_signins.entry(squad.to_owned()).or_default()
    }

    /// `squad`'s cards, for a screen attaching.
    pub(super) fn cli_sign_in_cards(&self, squad: &str) -> Vec<CliSignIn> {
        self.cli_signins.get(squad).map(|l| l.iter().map(|l| l.card.clone()).collect()).unwrap_or_default()
    }

    /// What `ticket` still waits on through CLI sign-in cards, one line each.
    pub(super) fn cli_sign_in_waits(&self, squad: &str, ticket: TicketId) -> Vec<String> {
        self.cli_signins.get(squad).map(|l| l.iter().filter(|l| l.card.ticket == Some(ticket) && l.card.phase.open()).map(|l| super::decide::one_line(&l.card.why)).collect()).unwrap_or_default()
    }

    /// Cards that still need the person, for the PM's note of what waits.
    pub(super) fn cli_sign_in_open(&self, squad: &str) -> Vec<CliSignIn> {
        self.cli_sign_in_cards(squad).into_iter().filter(|c| c.phase.open()).collect()
    }

    fn publish_card(&mut self, squad: &str, id: u64) {
        if let Some(card) = self.cli_signins(squad).iter().find(|l| l.card.id == id).map(|l| l.card.clone()) {
            self.publish(Event::CliSignInChanged { squad: squad.to_owned(), card });
        }
    }

    /// A member asks: checked, put on a card, and the sign-in started; it carries on meanwhile.
    pub(super) fn cli_sign_in(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let asked = cs::check(args)?;
        let plan = cs::plan(asked.kind, asked.profile.as_deref(), &asked.extra)?;
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        // The same ask again (this member, this tool and profile) replaces its card.
        let again: Vec<u64> = self.cli_signins(squad).iter().filter(|l| l.card.session == id && l.card.tool == asked.kind.word() && l.card.profile == asked.profile).map(|l| l.card.id).collect();
        for old in again {
            self.close_card(squad, old);
        }
        let card = CliSignIn {
            id: now_secs() * 1000 + self.cli_signins(squad).len() as u64,
            session: id,
            member: member.to_owned(),
            ticket,
            tool: asked.kind.word().to_owned(),
            name: asked.kind.name().to_owned(),
            profile: asked.profile.clone(),
            why: asked.why.clone(),
            url: None,
            user_code: None,
            wants_code: asked.kind.wants_code(),
            phase: Phase::Starting,
            who: None,
            detail: None,
            at: now_secs(),
            server: None,
        };
        let card_id = card.id;
        self.cli_signins(squad).push(Live { card, kind: asked.kind, plan, run: 0, codes: None, task: None, approving: None, server: None });
        self.run_cli_sign_in(squad, card_id);
        self.publish_card(squad, card_id);
        let whose = asked.profile.as_deref().map(|p| format!(" (profile {p})")).unwrap_or_default();
        Ok(format!(
            "The person is asked on a card to sign in to {}{whose}: equipe runs the tool's own cross-device sign-in here and \
shows them its page and code — you never see a code, a password or a token. When they have signed in you are told as whom, \
and carry on from there; if the link runs out they get a new one from the card. Meanwhile go on with what doesn't need it, \
or end your turn saying in a line what you're waiting for.",
            asked.kind.name()
        ))
    }

    /// Whether a sign-in card for outside server `server` on `kind`'s engine
    /// is up (whatever its phase): one card per server and engine, never two.
    pub(super) fn mcp_card_up(&mut self, squad: &str, server: &str, kind: Kind) -> bool {
        self.cli_signins(squad).iter().any(|l| l.kind == kind && l.server.as_deref() == Some(server))
    }

    /// An outside MCP server's sign-in on a card, started at once: `name` is
    /// how the card names it ("Mobbin (Claude Code)"), `session` and `member`
    /// the member it came up for (none: the person asked for it).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_mcp_card(&mut self, squad: &str, kind: Kind, plan: cs::Plan, server: &str, name: String, why: String, by: Option<(SessionId, String, Option<TicketId>)>) -> u64 {
        let (session, member, ticket) = by.unwrap_or((0, String::new(), None));
        let card = CliSignIn {
            id: now_secs() * 1000 + self.cli_signins(squad).len() as u64,
            session,
            member,
            ticket,
            tool: kind.word().to_owned(),
            name,
            profile: None,
            why,
            url: None,
            user_code: None,
            wants_code: kind.wants_code(),
            phase: Phase::Starting,
            who: None,
            detail: None,
            at: now_secs(),
            server: None,
        };
        let card = CliSignIn { server: Some(server.to_owned()), ..card };
        let card_id = card.id;
        self.cli_signins(squad).push(Live { card, kind, plan, run: 0, codes: None, task: None, approving: None, server: Some(server.to_owned()) });
        self.run_cli_sign_in(squad, card_id);
        self.publish_card(squad, card_id);
        crate::daemon_log!("equipe: {squad}: a sign-in card for the outside MCP server {server} on {} is up", kind.name());
        card_id
    }

    /// The person asked again for outside server `server`'s sign-in on
    /// `kind`'s engine while its card is up: the card is sent again (a screen
    /// puts it back in front of them), started over where it had ended.
    pub(super) fn reshow_mcp_card(&mut self, squad: &str, server: &str, kind: Kind) {
        let Some((id, phase)) = self.cli_signins(squad).iter().find(|l| l.kind == kind && l.server.as_deref() == Some(server)).map(|l| (l.card.id, l.card.phase)) else { return };
        if matches!(phase, Phase::Expired | Phase::Failed) {
            self.restart_cli_sign_in(squad, id);
        } else {
            self.publish_card(squad, id);
        }
    }

    /// The sign-in cards for outside server `server`, taken away (it left the squad).
    pub(super) fn close_mcp_cards(&mut self, squad: &str, server: &str) {
        let ids: Vec<u64> = self.cli_signins(squad).iter().filter(|l| l.server.as_deref() == Some(server)).map(|l| l.card.id).collect();
        for id in ids {
            self.close_card(squad, id);
        }
    }

    /// Start (or start again) the process behind card `id`.
    fn run_cli_sign_in(&mut self, squad: &str, id: u64) {
        let internal = self.internal.clone();
        let squad_s = squad.to_owned();
        let Some(live) = self.cli_signins(squad).iter_mut().find(|l| l.card.id == id) else { return };
        if let Some(task) = live.task.take() {
            task.abort();
        }
        live.run += 1;
        let (run, kind, plan) = (live.run, live.kind, live.plan.clone());
        let (tx, rx) = mpsc::unbounded_channel();
        live.codes = Some(tx);
        // equipe's own sign-in to an outside MCP server: no tool runs.
        if kind == Kind::EquipeMcp {
            let server = live.server.clone().unwrap_or_default();
            let (url, sse) = self.mcp_url_of(squad, &server).unwrap_or_default();
            let store = self.mcp_store();
            let Some(live) = self.cli_signins(squad).iter_mut().find(|l| l.card.id == id) else { return };
            live.task = Some(tokio::spawn(async move {
                let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(30)).build().unwrap_or_default();
                let seen = {
                    let (internal, squad) = (internal.clone(), squad_s.clone());
                    move |page: &str| {
                        let _ = internal.send(Internal::CliSignIn(Note::Seen { squad: squad.clone(), id, run, url: Some(page.to_owned()), code: None }));
                    }
                };
                let outcome = match crate::mcp_oauth::sign_in(&http, store, &squad_s, &server, &url, sse, cs::WAIT, rx, seen).await {
                    crate::mcp_oauth::Ended::Done => Outcome::Done,
                    crate::mcp_oauth::Ended::Expired => Outcome::Expired,
                    crate::mcp_oauth::Ended::Failed(why) => Outcome::Failed(why),
                };
                let _ = internal.send(Internal::CliSignIn(Note::Ended { squad: squad_s, id, run, outcome, who: None }));
            }));
            return;
        }
        live.task = Some(tokio::spawn(async move {
            let seen = {
                let (internal, squad) = (internal.clone(), squad_s.clone());
                move |url: Option<&str>, code: Option<&str>| {
                    let _ = internal.send(Internal::CliSignIn(Note::Seen { squad: squad.clone(), id, run, url: url.map(str::to_owned), code: code.map(str::to_owned) }));
                }
            };
            let outcome = cs::drive_in(kind, plan.cwd.as_deref(), &plan.program, &plan.args, &plan.env, cs::WAIT, rx, seen).await;
            // An MCP server's sign-in says nothing of as whom: it is checked again instead.
            let who = if outcome == Outcome::Done && !kind.is_mcp() { cs::whoami(&plan).await } else { None };
            let _ = internal.send(Internal::CliSignIn(Note::Ended { squad: squad_s, id, run, outcome, who }));
        }));
    }

    /// Take card `id` away everywhere, and its process with it.
    fn close_card(&mut self, squad: &str, id: u64) -> Option<CliSignIn> {
        let list = self.cli_signins(squad);
        let at = list.iter().position(|l| l.card.id == id)?;
        let mut live = list.remove(at);
        let card = live.card.clone();
        let approving = live.approving.take();
        // Its process stops with it.
        drop(live);
        if let Some(cli) = approving {
            self.device_card_ended(cli);
        }
        self.publish(Event::CliSignInClosed { squad: squad.to_owned(), id });
        Some(card)
    }

    pub(super) async fn cli_sign_in_note(&mut self, note: Note) {
        match note {
            Note::Seen { squad, id, run, url, code } => {
                let Some(live) = self.cli_signins(&squad).iter_mut().find(|l| l.card.id == id && l.run == run) else { return };
                live.card.url = url.clone();
                live.card.user_code = code.clone();
                if live.card.phase == Phase::Starting {
                    live.card.phase = Phase::Waiting;
                }
                let (session, kind) = (live.card.session, live.kind);
                self.publish_card(&squad, id);
                // Where the person already let equipe approve this tool's device sign-ins, it does.
                let by_itself = match kind {
                    Kind::Gh => Some("gh"),
                    Kind::AwsSso => Some("aws"),
                    _ => None,
                };
                if let (Some(url), Some(name)) = (url, by_itself)
                    && let Some(taken) = self.approve_device_for_card(session, name, url, code)
                    && let Some(live) = self.cli_signins(&squad).iter_mut().find(|l| l.card.id == id)
                {
                    live.approving = Some(taken);
                }
            }
            Note::Ended { squad, id, run, outcome, who } => {
                let Some(live) = self.cli_signins(&squad).iter_mut().find(|l| l.card.id == id && l.run == run) else { return };
                live.codes = None;
                live.task = None;
                let approving = live.approving.take();
                match outcome {
                    Outcome::Done => {
                        live.card.phase = Phase::SignedIn;
                        live.card.who = who.clone();
                    }
                    Outcome::Expired => live.card.phase = Phase::Expired,
                    Outcome::Failed(why) => {
                        live.card.phase = Phase::Failed;
                        live.card.detail = Some(why).filter(|w| !w.is_empty());
                    }
                }
                let card = live.card.clone();
                let server = live.server.clone();
                if let Some(name) = &server {
                    crate::daemon_log!("equipe: {squad}: the sign-in to the outside MCP server {name} ended: {:?}{}", card.phase, card.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default());
                }
                if let Some(cli) = approving {
                    self.device_card_ended(cli);
                }
                self.publish_card(&squad, id);
                if card.phase == Phase::SignedIn {
                    let (internal, swept) = (self.internal.clone(), squad.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(TICKED_FOR).await;
                        let _ = internal.send(Internal::CliSignIn(Note::Sweep { squad: swept, id }));
                    });
                    match server {
                        // Its members have it from their next turn: checked again, nobody woken.
                        Some(server) => self.check_mcp_servers(&squad, Some(&server), None),
                        None => self.cli_signed_in(&squad, &card).await,
                    }
                }
            }
            Note::Sweep { squad, id } => {
                if self.cli_signins(&squad).iter().any(|l| l.card.id == id && l.card.phase == Phase::SignedIn) {
                    self.close_card(&squad, id);
                }
            }
        }
    }

    /// The member goes on, told as whom.
    async fn cli_signed_in(&mut self, squad: &str, card: &CliSignIn) {
        let whose = card.profile.as_deref().map(|p| format!(" (profile {p})")).unwrap_or_default();
        let as_whom = card.who.as_deref().map(|w| format!(" as {w}")).unwrap_or_else(|| " (the tool didn't say as whom)".to_owned());
        let note = format!("The person signed in to {}{whose}{as_whom}. The tool keeps its own credentials; use it from your commands as usual.", card.name);
        if self.resume_waiting_on(squad, card.ticket.unwrap_or_default(), &[super::decide::one_line(&card.why)], &note).await {
            return;
        }
        self.set_going(card.session, format!("[equipe: {note} Carry on.]")).await;
    }

    /// The person pasted the code the page gave: to the process, and nowhere else.
    pub(super) fn submit_cli_sign_in_code(&mut self, squad: &str, id: u64, code: PastedCode) {
        let Some(live) = self.cli_signins(squad).iter_mut().find(|l| l.card.id == id) else { return };
        if !live.card.wants_code || code.0.trim().is_empty() {
            return;
        }
        let Some(tx) = live.codes.as_ref() else { return };
        if tx.send(code).is_ok() {
            live.card.phase = Phase::Checking;
            self.publish_card(squad, id);
        }
    }

    /// "Get a new link": the sign-in again, from the start.
    pub(super) fn restart_cli_sign_in(&mut self, squad: &str, id: u64) {
        let Some(live) = self.cli_signins(squad).iter_mut().find(|l| l.card.id == id) else { return };
        if !matches!(live.card.phase, Phase::Expired | Phase::Failed) {
            return;
        }
        let approving = live.approving.take();
        live.card.phase = Phase::Starting;
        live.card.url = None;
        live.card.user_code = None;
        live.card.detail = None;
        live.card.who = None;
        if let Some(cli) = approving {
            self.device_card_ended(cli);
        }
        self.run_cli_sign_in(squad, id);
        self.publish_card(squad, id);
    }

    /// The person won't sign in (now): the process stops, the card closes, the member hears it.
    pub(super) async fn cancel_cli_sign_in(&mut self, squad: &str, id: u64) {
        let mcp = self.cli_signins(squad).iter().find(|l| l.card.id == id).and_then(|l| l.server.clone().map(|s| (s, l.kind)));
        let Some(card) = self.close_card(squad, id) else { return };
        if card.phase == Phase::SignedIn {
            return;
        }
        // An outside server's sign-in, put off: not asked again for it until the person asks.
        if let Some((server, kind)) = mcp {
            self.mcp_put_off(squad, &server, kind);
            return;
        }
        let note = format!("The person didn't sign in to {} for now. Go on without it, or say in a line what it would let you do.", card.name);
        if self.resume_waiting_on(squad, card.ticket.unwrap_or_default(), &[super::decide::one_line(&card.why)], &note).await {
            return;
        }
        self.set_going(card.session, format!("[equipe: {note}]")).await;
    }
}
