//! Shareable links in the app (`equipe://open/<squad>/<kind>/<id>?host=…`,
//! or `https://equipe.im/o/#…`; the grammar is `equipe_core::protocol::deeplink`).
//!
//! Making one: the squad's name and the machine its daemon runs on — this
//! Mac's names (its Tailscale name, its host name) for a daemon here, the
//! name the window connects by for one over SSH — so whoever opens the link
//! can tell which of their daemons it is about.
//!
//! Opening one (a click in a chat app, `equipe open`, a link on the command
//! line, a pasted link): [`plan`] decides from the windows and machines the
//! app knows — go to the place in the window already on that daemon and squad
//! (which keeps its back and forward history), open another squad of an
//! attached daemon in a window of its own, offer a machine connected to before,
//! or say plainly which Mac and squad the link is for and how to connect.
//! Only [`handle`] and the `Shell` methods touch windows; the rest is plain.

use std::sync::OnceLock;
use std::time::Duration;

use equipe_core::protocol::SessionId;
use equipe_core::protocol::deeplink::{self, Daemon, DeepLink, Route, Target};
use equipe_core::t;
use gpui_kit::{App, ClipboardItem, Context, Window};

use crate::board::Menu;
use crate::sheet::{Ask, Button, Row, Sheet, Then};
use crate::state::Side;
use crate::target::{Endpoint, Remembered};
use crate::views::elsewhere::Goal;
use crate::views::sidebar::Place;
use crate::windows::{self, Opening};
use crate::{Screen, Shell, tailnet};

// ── this Mac's names ──

static THIS_MAC: OnceLock<Vec<String>> = OnceLock::new();

/// Read this Mac's names off the UI thread, before the first link is copied.
pub fn warm() {
    std::thread::spawn(|| {
        this_mac();
    });
}

/// The names this Mac answers to: its Tailscale (MagicDNS) name and host name,
/// then its own host name.
fn this_mac() -> &'static [String] {
    THIS_MAC.get_or_init(|| {
        let mut names = tailnet::load_self();
        let read = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                .filter(|name| !name.is_empty())
        };
        names.extend(read("/usr/sbin/scutil", &["--get", "LocalHostName"]));
        names.extend(read("hostname", &[]));
        let mut seen = Vec::new();
        names.retain(|n| {
            let key = n.to_ascii_lowercase();
            !seen.contains(&key) && {
                seen.push(key);
                true
            }
        });
        names
    })
}

/// Every name `endpoint`'s machine answers to, as far as this Mac knows.
pub fn names(endpoint: &Endpoint, peers: &[tailnet::Peer]) -> Vec<String> {
    match endpoint {
        Endpoint::Local { .. } => this_mac().to_vec(),
        Endpoint::Ssh(host) => {
            let mut out = Vec::new();
            if let Some(peer) = tailnet::peer_for(peers, &host.host) {
                out.extend([peer.dns.clone(), peer.short().to_owned(), peer.name.clone()]);
            }
            out.push(host.host.clone());
            out.retain(|n| !n.is_empty());
            out
        }
    }
}

/// The name a link carries for `endpoint`'s machine: a plain, shareable one
/// (a MagicDNS name, a host name; never a name with spaces or accents).
pub fn hint(endpoint: &Endpoint, peers: &[tailnet::Peer]) -> Option<String> {
    names(endpoint, peers).into_iter().find(|n| n.is_ascii() && !n.contains(' ') && !n.contains('@'))
}

// ── deciding ──

/// A machine the app knows: attached to by a window, or connected to before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub endpoint: Endpoint,
    pub names: Vec<String>,
    /// A window is connected to it now.
    pub attached: bool,
    /// The squad that window shows.
    pub squad: String,
    /// The squads it is known to have.
    pub squads: Vec<String>,
    /// The window (its place in the app's list) when there is one.
    pub window: Option<usize>,
}

/// What opening a link does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Go to the place in window `n`.
    Go(usize),
    /// Open `squad` of an attached machine in a window of its own.
    Open { endpoint: Endpoint, squad: String },
    /// Offer a machine connected to before.
    Switch { endpoint: Endpoint, squad: String, host: String },
    /// Say which Mac and squad it is for, and how to connect.
    Unknown { host: Option<String>, squad: String },
}

/// Decide what to do with `link` among the machines `known`.
pub fn plan(link: &DeepLink, known: &[Known]) -> Plan {
    let daemons: Vec<Daemon> = known.iter().map(|k| Daemon { names: k.names.clone(), squads: k.squads.clone(), attached: k.attached, current_squad: k.squad.clone() }).collect();
    let squad = link.squad.clone();
    match deeplink::route(link, &daemons) {
        Route::Here(i) => match known[i].window {
            Some(window) => Plan::Go(window),
            None => Plan::Open { endpoint: known[i].endpoint.clone(), squad },
        },
        Route::Squad(i) => Plan::Open { endpoint: known[i].endpoint.clone(), squad },
        Route::Switch(i) => Plan::Switch { endpoint: known[i].endpoint.clone(), squad, host: link.host.clone().unwrap_or_default() },
        Route::Unknown => Plan::Unknown { host: link.host.clone(), squad },
    }
}

/// Whether a window is still connecting to a machine the link may be about:
/// the answer is not in yet.
fn waiting(link: &DeepLink, known: &[Known]) -> bool {
    known.iter().any(|k| k.window.is_some() && !k.attached && link.host.as_deref().is_none_or(|h| k.names.iter().any(|n| deeplink::host_key(n) == deeplink::host_key(h))))
}

/// The machines the app knows now: each window's, this Mac, and those
/// connected to before.
fn known(cx: &App) -> Vec<Known> {
    let mut out: Vec<Known> = Vec::new();
    for (at, (_, shell)) in windows::shells().into_iter().enumerate() {
        let Some(shell) = shell.upgrade() else { continue };
        let shell = shell.read(cx);
        out.push(Known {
            names: names(&shell.endpoint, &shell.connect.peers),
            endpoint: shell.endpoint.clone(),
            attached: shell.model.attached(),
            squad: shell.model.squad.clone(),
            squads: shell.squads.clone(),
            window: Some(at),
        });
    }
    let peers: Vec<tailnet::Peer> = windows::shells().into_iter().filter_map(|(_, s)| s.upgrade()).flat_map(|s| s.read(cx).connect.peers.clone()).collect();
    let mut endpoints = vec![Endpoint::this_mac()];
    endpoints.extend(Remembered::load(&equipe_providers::credentials::equipe_home()).recent.into_iter().map(Endpoint::Ssh));
    for endpoint in endpoints {
        if !out.iter().any(|k| k.endpoint == endpoint) {
            out.push(Known { names: names(&endpoint, &peers), endpoint, attached: false, squad: String::new(), squads: Vec::new(), window: None });
        }
    }
    out
}

// ── opening ──

/// What a window of the app does with a link that was clicked, passed on the
/// command line or pasted: `text` is whatever holds it.
pub fn handle(text: &str, cx: &mut App) {
    let Some(link) = deeplink::parse(text) else { return };
    cx.spawn(async move |cx| {
        // A window that has not connected yet may be the very machine: wait for it.
        for _ in 0..40 {
            let decided = cx.update(|cx| {
                let known = known(cx);
                let plan = plan(&link, &known);
                (matches!(plan, Plan::Go(_) | Plan::Open { .. }) || !waiting(&link, &known)).then_some(plan)
            });
            if let Some(plan) = decided {
                cx.update(|cx| act(&link, plan, cx));
                return;
            }
            cx.background_executor().timer(Duration::from_millis(250)).await;
        }
        let plan = cx.update(|cx| plan(&link, &known(cx)));
        cx.update(|cx| act(&link, plan, cx));
    })
    .detach();
}

fn act(link: &DeepLink, plan: Plan, cx: &mut App) {
    cx.activate(true);
    let goal = Goal::Link(link.target.clone());
    match plan {
        Plan::Go(at) => {
            let Some((handle, shell)) = windows::shells().into_iter().nth(at) else { return };
            let Some(shell) = shell.upgrade() else { return };
            let target = link.target.clone();
            let _ = handle.update(cx, |_, window, cx| {
                window.activate_window();
                shell.update(cx, |shell, cx| shell.follow_link(target, window, cx));
            });
        }
        Plan::Open { endpoint, squad } => crate::views::elsewhere::open(Opening { endpoint: Some(endpoint), squad: Some(squad).filter(|s| !s.is_empty()), goal: Some(goal), chooser: false }, cx),
        Plan::Switch { endpoint, squad, host } => {
            let ask = switch_ask(link, endpoint, squad, &host);
            tell(cx, move |shell| shell.confirm = Some(ask));
        }
        Plan::Unknown { host, squad } => {
            let sheet = unknown_sheet(link, host.as_deref(), &squad);
            tell(cx, move |shell| shell.sheet = Some(sheet));
        }
    }
}

/// Show something in the window in front (else the first).
fn tell(cx: &mut App, show: impl FnOnce(&mut Shell) + 'static) {
    let target = windows::front(cx).map(|(h, s)| (h, s.downgrade())).or_else(|| windows::shells().into_iter().next());
    let Some((handle, shell)) = target else { return };
    let Some(shell) = shell.upgrade() else { return };
    let _ = handle.update(cx, |_, window, cx| {
        window.activate_window();
        shell.update(cx, |shell, cx| {
            show(shell);
            cx.notify();
        });
    });
}

/// What a link points at, in words: "ticket T-12", "Needs you".
pub fn what(target: &Target) -> String {
    match target {
        Target::Ticket(id) => t!("link.what_ticket", id = id),
        Target::Conversation(_) => t!("link.what_conversation").into(),
        Target::Member(name) => t!("link.what_member", name = name),
        Target::Decision(_) => t!("link.what_decision").into(),
        Target::NeedsYou => t!("link.what_needs_you").into(),
        Target::Status => t!("link.what_status").into(),
    }
}

/// "Open on mini?": a machine connected to before, in a window of its own.
pub fn switch_ask(link: &DeepLink, endpoint: Endpoint, squad: String, host: &str) -> Ask {
    Ask {
        title: t!("link.switch_title", host = host),
        body: vec![t!("link.switch_body", what = what(&link.target), squad = link.squad, host = host)],
        yes: t!("link.switch_yes").into(),
        then: Then::OpenWindow { endpoint, squad, goal: Goal::Link(link.target.clone()) },
        option: None,
    }
}

/// A link for a Mac the app doesn't know: which Mac, which squad, what to do.
pub fn unknown_sheet(link: &DeepLink, host: Option<&str>, squad: &str) -> Sheet {
    let lead = match host {
        Some(host) => t!("link.unknown_lead", what = what(&link.target), squad = squad, host = host),
        None => t!("link.unknown_lead_no_host", what = what(&link.target), squad = squad),
    };
    let mut rows = vec![Row::text(t!("link.unknown_squad"), squad)];
    if let Some(host) = host {
        rows.push(Row::text(t!("link.unknown_mac"), host));
    }
    Sheet {
        title: t!("link.unknown_title").into(),
        lead: Some(lead),
        rows,
        buttons: vec![Button::Copy(t!("link.copy").into(), link.uri())],
        foot: Some(match host {
            Some(host) => t!("link.unknown_how", host = host),
            None => t!("link.unknown_how_no_host").into(),
        }),
    }
}

// ── what a window does ──

/// How "Copy link" copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyAs {
    /// `equipe://open/…`
    Link,
    /// `T-2391 title · equipe://open/…`
    Text,
    /// `https://equipe.im/o/#…`
    Web,
}

/// What copying puts on the clipboard.
pub fn copied(link: &DeepLink, title: &str, how: CopyAs) -> String {
    match how {
        CopyAs::Link => link.uri(),
        CopyAs::Text => link.as_text(title),
        CopyAs::Web => link.web(),
    }
}

impl Shell {
    /// A link to `target` in this window's squad, on this window's machine.
    pub fn link_to(&self, target: Target) -> DeepLink {
        DeepLink::new(self.model.squad.clone(), target, hint(&self.endpoint, &self.connect.peers))
    }

    /// What `target` is called here: a ticket's number and title, a
    /// conversation's name, a member's name — what "Copy as text" says before the link.
    pub fn link_title(&self, target: &Target) -> String {
        let model = &self.model;
        match target {
            Target::Ticket(id) => model.tickets.iter().find(|t| t.id == *id).map_or_else(|| format!("T-{id}"), |t| format!("T-{id} {}", t.title)),
            Target::Conversation(session) => model.conversations().into_iter().find(|l| l.session == *session).and_then(|l| l.title.clone()).unwrap_or_else(|| t!("sidebar.new_conversation").into()),
            Target::Member(name) => name.clone(),
            Target::Decision(id) => model.asked.get(*id).and_then(|set| set.questions.first().map(|q| q.question.clone())).unwrap_or_else(|| t!("link.what_decision").into()),
            Target::NeedsYou => t!("link.title_needs_you", squad = model.squad),
            Target::Status => t!("link.title_status", squad = model.squad),
        }
    }

    /// Copy the link `board.link` is open for.
    pub fn copy_link(&mut self, how: CopyAs, cx: &mut Context<Self>) {
        let Some(menu) = self.board.link.clone() else { return };
        self.copy_link_to(menu.key, menu.target, how, cx);
    }

    /// Copy a link to `target` on the clipboard, and say so (a check on the
    /// control `key`, a line under the composer).
    pub fn copy_link_to(&mut self, key: &'static str, target: Target, how: CopyAs, cx: &mut Context<Self>) {
        let text = copied(&self.link_to(target.clone()), &self.link_title(&target), how);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.board.link_copied = Some((key, std::time::Instant::now()));
        self.say(match how {
            CopyAs::Text => t!("link.copied_text"),
            CopyAs::Link | CopyAs::Web => t!("link.copied"),
        });
        cx.notify();
    }

    /// Dev tool (`snapshot.rs`): ticket `id`'s "Copy link" menu, open.
    #[cfg(feature = "snapshot")]
    pub fn snapshot_link_menu(&mut self, id: u32, cx: &mut Context<Self>) {
        let target = Target::Ticket(id);
        let title = self.link_title(&target);
        self.board.link = Some(crate::views::linkshare::LinkMenu { key: "ticket-link", target, title });
        self.board.popover = Some(crate::views::board::Popover::Link);
        cx.notify();
    }

    /// ⌘⇧C: copy the link of the ticket open (a page, beside the conversation)
    /// or picked on the board.
    pub fn copy_ticket_link(&mut self, cx: &mut Context<Self>) {
        let id = match self.screen {
            Screen::Board => self.picked().map(|t| t.id).or(self.board_ticket),
            Screen::Chat => self.model.opened,
            _ => None,
        };
        if let Some(id) = id {
            self.copy_link_to("ticket-link", Target::Ticket(id), CopyAs::Link, cx);
        }
    }

    /// A link was opened and this window is the one: go to its place, once
    /// the daemon has said what there is.
    pub fn follow_link(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.goal = Some(Goal::Link(target));
        self.reach_goal(window, cx);
    }

    /// Show the place a link names, in this window (so back and forward keep it).
    pub fn open_link_target(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Ticket(id) => {
                if self.model.tickets.iter().any(|t| t.id == id) {
                    self.open_ticket_row(id, window, cx);
                } else {
                    self.say(t!("link.gone_ticket", id = id));
                }
            }
            Target::Conversation(session) => {
                if self.model.conversations().iter().any(|l| l.session == session) {
                    self.open_side(Side::Conversation(session), window, cx);
                } else {
                    self.say(t!("link.gone_conversation"));
                }
            }
            Target::Member(name) => match self.model.open_member(&name).map(|m| m.session) {
                Some(session) => self.select_member(session, window, cx),
                None => self.say(t!("link.gone_member", name = name)),
            },
            Target::Decision(id) => match self.model.asked.get(id).map(|set| (set.session, set.ticket)) {
                Some((session, ticket)) => self.show_asked(session, ticket, window, cx),
                None => self.say(t!("link.gone_decision")),
            },
            Target::NeedsYou => self.go_to(Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }, window, cx),
            Target::Status => self.go_to(Place { screen: Screen::Status, menu: Menu::Inbox, member: None, ticket: None }, window, cx),
        }
    }

    /// Where a member's question is answered: the conversation that asked (a
    /// member's, or one the person opened), else the ticket it is about.
    fn show_asked(&mut self, session: SessionId, ticket: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.members.iter().any(|m| m.session == session) {
            self.select_member(session, window, cx);
        } else if self.model.conversations().iter().any(|l| l.session == session) {
            self.open_side(Side::Conversation(session), window, cx);
        } else if let Some(ticket) = ticket {
            self.open_ticket_row(ticket, window, cx);
        } else {
            self.go_to(Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::SshHost;

    fn ssh(host: &str) -> Endpoint {
        Endpoint::Ssh(SshHost::parse(host).unwrap())
    }

    fn window(endpoint: Endpoint, names: &[&str], squad: &str, squads: &[&str], at: usize) -> Known {
        Known { endpoint, names: names.iter().map(|s| (*s).into()).collect(), attached: true, squad: squad.into(), squads: squads.iter().map(|s| (*s).into()).collect(), window: Some(at) }
    }

    fn known_only(endpoint: Endpoint, names: &[&str]) -> Known {
        Known { endpoint, names: names.iter().map(|s| (*s).into()).collect(), attached: false, squad: String::new(), squads: Vec::new(), window: None }
    }

    fn link(squad: &str, host: Option<&str>) -> DeepLink {
        DeepLink::new(squad, Target::Ticket(2391), host.map(Into::into))
    }

    #[test]
    fn a_link_to_the_daemon_and_squad_a_window_shows_goes_to_that_window() {
        let known = [window(Endpoint::this_mac(), &["macbook"], "web", &["web"], 0), window(ssh("me@mini.tail3383cf.ts.net"), &["mini.tail3383cf.ts.net"], "web", &["web"], 1)];
        assert_eq!(plan(&link("web", Some("mini")), &known), Plan::Go(1), "the host decides between two squads called web");
        assert_eq!(plan(&link("web", Some("macbook.local")), &known), Plan::Go(0));
        assert_eq!(plan(&link("web", None), &known), Plan::Go(0), "no host: the first window on the squad");
    }

    #[test]
    fn another_squad_of_an_attached_daemon_opens_in_a_window_of_its_own() {
        let known = [window(ssh("mini"), &["mini"], "web", &["web", "docs"], 0)];
        assert_eq!(plan(&link("docs", Some("mini")), &known), Plan::Open { endpoint: ssh("mini"), squad: "docs".into() });
    }

    #[test]
    fn a_known_machine_not_attached_is_offered() {
        let known = [window(Endpoint::this_mac(), &["macbook"], "web", &["web"], 0), known_only(ssh("me@mini.tail3383cf.ts.net"), &["mini.tail3383cf.ts.net"])];
        assert_eq!(plan(&link("web", Some("mini")), &known), Plan::Switch { endpoint: ssh("me@mini.tail3383cf.ts.net"), squad: "web".into(), host: "mini".into() });
    }

    #[test]
    fn an_unknown_machine_is_named_not_guessed() {
        let known = [window(Endpoint::this_mac(), &["macbook"], "web", &["web"], 0)];
        assert_eq!(plan(&link("web", Some("studio")), &known), Plan::Unknown { host: Some("studio".into()), squad: "web".into() });
        assert_eq!(plan(&link("nope", None), &known), Plan::Unknown { host: None, squad: "nope".into() });
    }

    #[test]
    fn a_window_still_connecting_to_the_machine_is_waited_for() {
        let mut connecting = window(ssh("mini"), &["mini"], "", &[], 0);
        connecting.attached = false;
        assert!(waiting(&link("web", Some("mini.tail3383cf.ts.net")), &[connecting.clone()]));
        assert!(!waiting(&link("web", Some("studio")), &[connecting.clone()]), "another machine's link isn't held up by it");
        assert!(!waiting(&link("web", Some("mini")), &[known_only(ssh("mini"), &["mini"])]), "a machine with no window isn't connecting");
    }

    #[test]
    fn copying_makes_the_link_its_text_or_its_web_form() {
        let l = DeepLink::new("web", Target::Ticket(2391), Some("mini".into()));
        assert_eq!(copied(&l, "T-2391 구글시트 4개 공유하기", CopyAs::Link), "equipe://open/web/ticket/T-2391?host=mini");
        assert_eq!(copied(&l, "T-2391 구글시트 4개 공유하기", CopyAs::Text), "T-2391 구글시트 4개 공유하기 · equipe://open/web/ticket/T-2391?host=mini");
        assert_eq!(copied(&l, "x", CopyAs::Web), "https://equipe.im/o/#web/ticket/T-2391?host=mini");
    }

    #[test]
    fn a_shareable_name_is_plain_ascii() {
        let peers = tailnet::parse(r#"{"Peer": {"b": {"HostName": "Abel의 Mac mini", "DNSName": "abel-macmini.tail5abbbf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.106.17.55"], "Online": true}}}"#);
        assert_eq!(hint(&ssh("me@abel-macmini"), &peers).as_deref(), Some("abel-macmini.tail5abbbf.ts.net"), "the MagicDNS name, not the alias typed");
        assert_eq!(hint(&ssh("me@box.example.com"), &peers).as_deref(), Some("box.example.com"), "no tailnet: what it connects by");
        assert_eq!(hint(&ssh("me@100.1.2.3"), &[]).as_deref(), Some("100.1.2.3"));
    }
}
