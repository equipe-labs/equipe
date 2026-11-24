//! "Connections" on the Logins page: the outside services this squad uses
//! (`equipe_core::connections::CATALOG`, plus any other service a kept key is
//! for) — for each, whether a key is saved, whether the members' browser is
//! signed in (from the saved logins), when a key was last used — and the
//! buttons that start the same flows the members use: Connect (the key card,
//! `Command::RequestServiceKey`), Sign in (the in-app "Sign in to a site"
//! flow) and Disconnect (delete the saved key; signing out is the site's own).
//!
//! A service members use through its official MCP server (Mobbin, 21st.dev)
//! has its own row, with one obvious action for where it stands
//! ([`McpAction`]): Not connected → Connect (and the plan's page); Needs
//! sign-in → Sign in; Needs its key → Add key; Connected → a quiet
//! "Connected ✓" and a ⋯ menu (Sign in again or Replace key, Disconnect).
//! Connect, Sign in and Add key open the sign-in card or the key card as a
//! sheet over the page — never somewhere up the page out of sight — or say
//! why it didn't come; when it is done the sheet closes and the row lights
//! up in its new state. Any other MCP server is added by its address at the
//! end of the section.
//!
//! The list carries facts only; a key's value or a token never comes to this page.

use std::time::{Duration, Instant};

use equipe_core::connections::{self, CATALOG, Service, SignedIn};
use equipe_core::protocol::Command;
use equipe_core::protocol::mcp_servers::{McpAuth, McpServer, McpServerStatus, McpState};
use equipe_core::protocol::secrets::SquadSecret;
use equipe_core::protocol::vault::Login;
use equipe_core::squad_mcp;
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::brands::Mark;
use crate::icons::{Icon, icon};
use crate::theme::{CONTROL, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// What the section remembers between draws: the row whose Disconnect waits
/// for its second click, the row whose ⋯ menu is open, the sheet up, the row
/// lit for a moment, and the boxes of "Add an MCP server".
pub struct ConnectionsUi {
    pub confirm: Option<String>,
    pub menu: Option<String>,
    pub sheet: Option<Sheet>,
    /// A row just connected, lit until this time.
    pub lit: Option<(String, Instant)>,
    pub mcp_name: Entity<InputState>,
    pub mcp_url: Entity<InputState>,
    pub mcp_roles: Entity<InputState>,
    /// What was wrong with the server last typed in.
    pub mcp_problem: Option<String>,
}

impl ConnectionsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            confirm: None,
            menu: None,
            sheet: None,
            lit: None,
            mcp_name: cx.new(|cx| InputState::new(window, cx).placeholder(t!("connections.mcp_name_placeholder"))),
            mcp_url: cx.new(|cx| InputState::new(window, cx).placeholder(t!("connections.mcp_url_placeholder"))),
            mcp_roles: cx.new(|cx| InputState::new(window, cx).placeholder(t!("connections.mcp_roles_placeholder"))),
            mcp_problem: None,
        }
    }

    /// The boxes' words follow the language.
    pub fn relocalize(&self, window: &mut Window, cx: &mut Context<Shell>) {
        self.mcp_name.update(cx, |s, cx| s.set_placeholder(t!("connections.mcp_name_placeholder"), window, cx));
        self.mcp_url.update(cx, |s, cx| s.set_placeholder(t!("connections.mcp_url_placeholder"), window, cx));
        self.mcp_roles.update(cx, |s, cx| s.set_placeholder(t!("connections.mcp_roles_placeholder"), window, cx));
    }
}

/// How long "no answer" waits for a card to come up.
const ANSWER_WAIT: Duration = Duration::from_secs(20);
/// How long a row stays lit once connected.
const LIT_FOR: Duration = Duration::from_secs(3);

/// The sheet over the page: the card the person asked for from a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    /// The row it is for (lit when it is done).
    pub row: String,
    pub what: Asked,
    pub at: Instant,
    /// Its card came up (its going away then means it is done or put off).
    pub seen: bool,
    /// The row stood connected when it was asked (a "Sign in again").
    pub was_connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// A kept key's card (by the key's name).
    Key { secret: String },
    /// An MCP server's sign-in card (by the server's name).
    SignIn { server: String },
}

/// The one obvious thing to do on an MCP row, and what else its ⋯ menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpAction {
    Connect,
    SignIn,
    AddKey,
    /// It couldn't be reached (the server, not the sign-in): look again.
    Retry,
    /// Connected: nothing to do; the menu has Sign in again / Replace key and Disconnect.
    Connected,
    /// Being looked at, or off: nothing to press but the menu.
    Quiet,
}

/// The row's action for where its server stands.
pub fn mcp_action(mcp: &McpRowState) -> McpAction {
    match (mcp.server, mcp.state) {
        (None, _) => McpAction::Connect,
        (Some((_, false)), _) => McpAction::Quiet,
        (Some(_), Some(McpState::Connected)) => McpAction::Connected,
        (Some(_), Some(McpState::Failing)) => McpAction::Retry,
        (Some((true, _)), Some(McpState::NeedsKey | McpState::NeedsSignIn)) => McpAction::AddKey,
        (Some((false, _)), Some(McpState::NeedsSignIn | McpState::NeedsKey)) => McpAction::SignIn,
        (Some(_), _) => McpAction::Quiet,
    }
}

/// What [`mcp_action`] reads of a row: its server, if connected (keyed, on), and where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpRowState {
    pub server: Option<(bool, bool)>,
    pub state: Option<McpState>,
}

impl McpRow {
    fn state(&self) -> McpRowState {
        McpRowState { server: self.server.as_ref().map(|s| (matches!(s.auth, McpAuth::Key { .. }), s.enabled)), state: self.status.as_ref().map(McpServerStatus::overall) }
    }
}

/// One row: a catalog service, another service a kept key is for, or an MCP server of the squad's own.
struct Row {
    /// The catalog id, the service text of a key outside the catalog, or `mcp:<name>`.
    id: String,
    label: String,
    service: Option<&'static Service>,
    status: connections::Status,
    /// The key names kept for it (for Disconnect), whichever way its service was written.
    keys: Vec<String>,
    /// An MCP row: the squad's server (none: not connected yet) and where it stands.
    mcp: Option<McpRow>,
}

#[derive(Clone)]
struct McpRow {
    server: Option<McpServer>,
    status: Option<McpServerStatus>,
}

fn rows(shell: &Shell) -> Vec<Row> {
    let mut out = rows_of(shell.model.squad.as_str(), &shell.model.squad_secrets, &shell.model.logins);
    add_mcp_rows(&mut out, &shell.model.mcp_servers, &shell.model.mcp_status);
    out
}

/// The squad's MCP servers on their rows: a catalog one (added from the
/// catalog, or by its official address) on its service's row, any other on a
/// row of its own after the catalog's.
fn add_mcp_rows(rows: &mut Vec<Row>, servers: &[McpServer], status: &[McpServerStatus]) {
    let status_of = |name: &str| status.iter().find(|s| s.name == name).cloned();
    let mut shown: Vec<&str> = Vec::new();
    for row in rows.iter_mut().filter(|r| r.service.is_some_and(Service::is_mcp)) {
        // The one added from the catalog first, else one added by the same address.
        let server = servers
            .iter()
            .find(|s| s.catalog.as_deref() == Some(row.id.as_str()))
            .or_else(|| servers.iter().find(|s| squad_mcp::catalog_service(s).is_some_and(|c| c.id == row.id)));
        if let Some(server) = server {
            shown.push(server.name.as_str());
        }
        let server = server.cloned();
        let status = server.as_ref().and_then(|s| status_of(&s.name));
        row.mcp = Some(McpRow { server, status });
    }
    for server in servers.iter().filter(|s| !shown.contains(&s.name.as_str())) {
        rows.push(Row {
            id: format!("mcp:{}", server.name),
            label: server.name.clone(),
            service: None,
            status: connections::Status { id: "", keys: Vec::new(), signed_in: SignedIn::Unknown, last_used: None },
            keys: Vec::new(),
            mcp: Some(McpRow { server: Some(server.clone()), status: status_of(&server.name) }),
        });
    }
}

fn rows_of(squad: &str, secrets: &[SquadSecret], logins: &[Login]) -> Vec<Row> {
    let mut out: Vec<Row> = CATALOG
        .iter()
        .map(|s| {
            let status = connections::status(s, squad, secrets, logins);
            Row { id: s.id.to_owned(), label: s.label.to_owned(), service: Some(s), keys: status.keys.clone(), status, mcp: None }
        })
        .collect();
    // Services outside the catalog the squad kept a key for: one row each.
    for secret in secrets.iter().filter(|k| k.squad == squad && Service::of(&k.service).is_none()) {
        match out.iter_mut().find(|r| r.service.is_none() && r.id == secret.service) {
            Some(row) => {
                row.keys.push(secret.name.clone());
                row.status.keys.push(secret.name.clone());
                if secret.last_used.as_ref().map(|u| u.at) > row.status.last_used.as_ref().map(|u| u.at) {
                    row.status.last_used = secret.last_used.clone();
                }
            }
            None => out.push(Row {
                id: secret.service.clone(),
                label: secret.service.clone(),
                service: None,
                status: connections::Status { id: "", keys: vec![secret.name.clone()], signed_in: SignedIn::Unknown, last_used: secret.last_used.clone() },
                keys: vec![secret.name.clone()],
                mcp: None,
            }),
        }
    }
    out
}

/// The row's tile: its logo or monogram, and where it has none of Simple
/// Icons', the service's own site icon once that is fetched (`siteicon`).
fn mark_tile(p: Palette, row: &Row, cx: &mut Context<Shell>) -> Div {
    let mark = crate::brands::mark_of(row.service, &row.label);
    let icon = match mark {
        Mark::Glyph { .. } => None,
        Mark::Monogram { .. } => {
            let wanted = match (row.service, row.mcp.as_ref().and_then(|m| m.server.as_ref())) {
                (Some(service), _) => crate::siteicon::for_catalog(service.id),
                (None, Some(server)) => crate::siteicon::for_server_url(&server.url),
                (None, None) => crate::siteicon::for_service_text(&row.id),
            };
            wanted.and_then(|w| crate::siteicon::icon(&w, cx))
        }
    };
    crate::brands::tile_with(p, &mark, icon.as_deref())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn chip(p: Palette, text: String, tone: Rgba) -> Div {
    div().flex_none().max_w(px(260.)).h(px(22.)).px_2().flex().items_center().rounded_full().bg(p.wash).text_size(px(12.)).text_color(tone).whitespace_nowrap().overflow_hidden().child(text)
}

/// The section: a heading, a line on what it is, then a card with a row for each service.
pub fn section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mut body = v_flex().w_full().rounded(px(R_LG)).border_1().border_color(p.hair).bg(p.raised).overflow_hidden();
    // The mailbox verification codes are read from, first: it serves every site.
    body = body.child(super::verify::mailbox_row(shell, cx));
    for row in rows(shell).iter() {
        body = body.child(row_view(shell, row, true, cx));
    }
    // A key card or a sign-in a member asked for, not opened here: one line
    // that opens it as the sheet (a card is never drawn up the page, out of sight).
    let waiting: Vec<(String, Asked, String)> = shell
        .model
        .secret_asks
        .iter()
        .filter_map(|a| a.store.as_ref().map(|st| (st.service.clone(), Asked::Key { secret: st.name.clone() }, connections::title_of(&st.name, &st.service))))
        .chain(shell.model.cli_sign_ins.iter().filter(|c| super::cli_signin::is_mcp(c)).map(|c| (c.server.clone().unwrap_or_else(|| c.why.clone()), Asked::SignIn { server: c.server.clone().unwrap_or_else(|| c.why.clone()) }, super::cli_signin::title(c))))
        .filter(|(_, asked, _)| shell.logins_ui.connections.sheet.as_ref().is_none_or(|s| s.what != *asked))
        .collect();
    let waiting = waiting.into_iter().enumerate().map(|(n, (row, asked, title))| {
        h_flex()
            .id(("connections-waiting", n))
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .child(icon(Icon::Lock, 13., p.accent))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(p.text).child(title))
            .child(ui::button(p, ("connections-waiting-open", n), Kind::Primary, t!("connections.open_card")).on_click(cx.listener(move |this, _, _, cx| this.connections_show(&row, asked.clone(), cx))))
            .into_any_element()
    });
    v_flex()
        .id("logins-connections")
        .w_full()
        .gap_2()
        .child(h_flex().w_full().px_1().gap_2().items_baseline().child(div().id("connections-title").text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).role(Role::Heading).child(t!("connections.title"))))
        .child(div().w_full().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.subtitle", machine = shell.endpoint.label())))
        .children(super::secrets::saved_line(shell))
        .children(waiting)
        .child(body)
        .child(add_mcp_form(shell, cx))
        .into_any_element()
}

/// "Add an MCP server": its name, its https address and (optionally) the roles that get it.
fn add_mcp_form(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.logins_ui.connections;
    let field = |input: &Entity<InputState>, width: Option<f32>| {
        let b = div().h(px(CONTROL)).px_3().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.code_bg).text_size(px(13.)).child(Input::new(input).appearance(false).bordered(false));
        match width {
            Some(w) => b.w(px(w)).flex_none(),
            None => b.flex_1().min_w(px(180.)),
        }
    };
    v_flex()
        .id("connections-mcp-add")
        .w_full()
        .gap_2()
        .px_1()
        .pt_2()
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("connections.mcp_add_title")))
        .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.mcp_add_note")))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .flex_wrap()
                .items_center()
                .child(field(&ui.mcp_name, Some(150.)))
                .child(field(&ui.mcp_url, None))
                .child(field(&ui.mcp_roles, Some(230.)))
                .child(ui::button(p, "conn-mcp-add", Kind::Secondary, t!("connections.mcp_add")).icon(Icon::Plus).on_click(cx.listener(|this, _, window, cx| this.connections_add_mcp(window, cx)))),
        )
        .children(ui.mcp_problem.clone().map(|why| div().text_size(px(12.)).text_color(p.danger()).child(why)))
        .into_any_element()
}

/// Where an MCP server stands, in a chip's words and tone.
fn mcp_chip(p: Palette, mcp: &McpRow) -> (String, Rgba) {
    match (&mcp.server, mcp.status.as_ref().map(McpServerStatus::overall)) {
        (None, _) => (t!("connections.mcp_not_connected").to_owned(), p.muted),
        (Some(s), _) if !s.enabled => (t!("connections.mcp_off").to_owned(), p.muted),
        (Some(_), Some(McpState::Connected)) => (t!("connections.mcp_connected").to_owned(), p.text2),
        (Some(_), Some(McpState::NeedsKey)) => (t!("connections.mcp_needs_key").to_owned(), p.danger()),
        (Some(s), Some(McpState::NeedsSignIn)) if matches!(s.auth, McpAuth::Key { .. }) => (t!("connections.mcp_needs_key").to_owned(), p.danger()),
        (Some(_), Some(McpState::NeedsSignIn)) => (t!("connections.mcp_needs_sign_in").to_owned(), p.danger()),
        (Some(_), Some(McpState::Failing)) => (t!("connections.mcp_failing").to_owned(), p.danger()),
        (Some(_), _) => (t!("connections.mcp_checking").to_owned(), p.muted),
    }
}

fn row_view(shell: &Shell, row: &Row, divider: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let status = &row.status;
    let id = row.id.clone();
    let element = |prefix: &str| SharedString::from(format!("{prefix}-{id}"));
    let used = row.service.and_then(|s| super::secrets::service_words(s.id, "use")).or_else(|| row.mcp.as_ref().and_then(|m| m.server.as_ref()).map(McpServer::place));
    if let Some(mcp) = &row.mcp {
        return mcp_row_view(shell, row, mcp, used, divider, cx);
    }
    let key_chip = if row.keys.is_empty() { chip(p, t!("connections.key_none").to_owned(), p.muted) } else { chip(p, t!("connections.key_saved", names = row.keys.join(", ")), p.text2) };
    let mut chips = h_flex().flex_none().gap_2().items_center().child(key_chip);
    if row.service.is_some() {
        let (text, tone) = match status.signed_in {
            SignedIn::Yes => (t!("connections.signed_in"), p.text2),
            SignedIn::NeedsAttention => (t!("connections.signin_attention"), p.danger()),
            SignedIn::Saved => (t!("connections.signin_saved"), p.muted),
            SignedIn::Unknown => (t!("connections.signin_unknown"), p.muted),
        };
        chips = chips.child(chip(p, text.to_owned(), tone));
    }
    let last = match &status.last_used {
        Some(u) => t!("connections.last_used", member = u.member, when = crate::activity::ago(now_secs().saturating_sub(u.at))),
        None if row.keys.is_empty() => String::new(),
        None => t!("connections.never_used").to_owned(),
    };
    let mut actions = h_flex().flex_none().gap_2().items_center();
    if let Some(service) = row.service {
        if service.takes_key() && row.keys.is_empty() {
            let sid = service.id.to_owned();
            actions = actions.child(ui::button(p, element("conn-connect"), Kind::Primary, t!("connections.connect")).icon(Icon::Plug).on_click(cx.listener(move |this, _, _, cx| this.connections_connect(&sid, cx))));
        }
        if let Some(url) = service.sign_in_target() {
            actions = actions.child(ui::button(p, element("conn-signin"), Kind::Secondary, t!("connections.sign_in")).icon(Icon::Globe).on_click(cx.listener(move |this, _, _, cx| this.sign_in_to(url, cx))));
        }
    }
    if !row.keys.is_empty() {
        if shell.logins_ui.connections.confirm.as_deref() == Some(row.id.as_str()) {
            let yes_id = row.id.clone();
            actions = actions
                .child(ui::button(p, element("conn-disconnect-no"), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.connections.confirm = None;
                    cx.notify();
                })))
                .child(ui::button(p, element("conn-disconnect-yes"), Kind::Danger, t!("connections.disconnect_yes")).on_click(cx.listener(move |this, _, _, cx| this.connections_disconnect(&yes_id, cx))));
        } else {
            let ask_id = row.id.clone();
            actions = actions.child(ui::button(p, element("conn-disconnect"), Kind::Ghost, t!("connections.disconnect")).icon(Icon::Trash).on_click(cx.listener(move |this, _, _, cx| {
                this.logins_ui.connections.confirm = Some(ask_id.clone());
                cx.notify();
            })));
        }
    }
    let confirming = shell.logins_ui.connections.confirm.as_deref() == Some(row.id.as_str()) && !row.keys.is_empty();
    v_flex()
        .id(element("conn-row"))
        .w_full()
        .when(divider, |b| b.border_t_1().border_color(p.hair))
        .child(
            h_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_3()
                .items_center()
                .child(mark_tile(p, row, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(row.label.clone()))
                        .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(used.unwrap_or_default()))
                        .child(h_flex().w_full().pt_1().gap_2().flex_wrap().items_center().child(chips).children((!last.is_empty()).then(|| div().text_size(px(11.5)).text_color(p.muted).child(last)))),
                )
                .child(actions),
        )
        .children(confirming.then(|| div().w_full().px_4().pb_3().pl(px(16. + 32. + 12.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.disconnect_note"))))
        .into_any_element()
}

/// An MCP server's row: where it stands, for which roles, one obvious
/// action, and — once connected — a ⋯ menu for the rest.
fn mcp_row_view(shell: &Shell, row: &Row, mcp: &McpRow, used: Option<String>, divider: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_state = &shell.logins_ui.connections;
    let id = row.id.clone();
    let element = |prefix: &str| SharedString::from(format!("{prefix}-{id}"));
    let action = mcp_action(&mcp.state());
    let keyed = mcp.server.as_ref().is_some_and(|s| matches!(s.auth, McpAuth::Key { .. }));
    let mut chips = h_flex().flex_none().gap_2().items_center();
    if action != McpAction::Connected {
        let (text, tone) = mcp_chip(p, mcp);
        chips = chips.child(chip(p, text, tone));
    }
    let roles = match (&mcp.server, row.service.and_then(|s| s.mcp)) {
        (Some(s), _) if s.roles.is_empty() => Some(t!("connections.mcp_for_everyone").to_owned()),
        (Some(s), _) => Some(t!("connections.mcp_for", roles = s.roles.join(", "))),
        (None, Some(m)) => Some(t!("connections.mcp_for", roles = m.roles.join(", "))),
        (None, None) => None,
    };
    if let Some(roles) = roles {
        chips = chips.child(chip(p, roles, p.muted));
    }
    // What its plan needs, from the catalog's words: only before connecting.
    let plan = row.service.and_then(|s| super::secrets::service_words(s.id, "hint"));
    let detail = mcp.status.as_ref().filter(|s| matches!(s.overall(), McpState::Failing | McpState::NeedsKey)).and_then(|s| s.detail().map(str::to_owned));
    let mut actions = h_flex().flex_none().gap_2().items_center();
    let name = mcp.server.as_ref().map(|s| s.name.clone());
    let asked_now = |what: &Asked| ui_state.sheet.as_ref().is_some_and(|s| s.what == *what);
    match action {
        McpAction::Connect => {
            if let Some(service) = row.service {
                let sid = service.id.to_owned();
                actions = actions.child(ui::button(p, element("conn-mcp-connect"), Kind::Primary, t!("connections.connect")).icon(Icon::Plug).on_click(cx.listener(move |this, _, _, cx| this.connections_connect_mcp(&sid, cx))));
                if let Some(m) = service.mcp {
                    let plans = m.sign_up_url.to_owned();
                    actions = actions.child(ui::button(p, element("conn-mcp-plans"), Kind::Ghost, t!("connections.mcp_plans")).icon(Icon::External).on_click(cx.listener(move |_, _, _, cx| cx.open_url(&plans))));
                }
            }
        }
        McpAction::SignIn | McpAction::AddKey => {
            let n = name.clone().unwrap_or_default();
            let (label, glyph) = if action == McpAction::AddKey { (t!("connections.mcp_add_key"), Icon::Lock) } else { (t!("connections.sign_in"), Icon::Globe) };
            let busy = mcp.server.as_ref().is_some_and(|s| asked_now(&asked_for(s)));
            actions = actions.child(ui::button(p, element("conn-mcp-signin"), Kind::Primary, label).icon(glyph).disabled(busy).on_click(cx.listener(move |this, _, _, cx| this.connections_mcp_sign_in(&n, cx))));
        }
        McpAction::Retry => {
            actions = actions.child(ui::button(p, element("conn-mcp-retry"), Kind::Secondary, t!("connections.mcp_try_again")).icon(Icon::Reload).on_click(cx.listener(|this, _, _, cx| {
                let squad = this.model.squad.clone();
                this.send_command(Command::CheckMcpServers { squad });
                cx.notify();
            })));
        }
        McpAction::Connected => {
            actions = actions.child(h_flex().id(element("conn-mcp-ok")).gap_1().items_center().text_size(px(13.)).text_color(p.text2).child(t!("connections.mcp_connected")).child(icon(Icon::Check, 13., p.accent)));
        }
        McpAction::Quiet => {}
    }
    if mcp.server.is_some() {
        let ask_id = row.id.clone();
        let open = ui_state.menu.as_deref() == Some(row.id.as_str());
        actions = actions.child(ui::icon_button(p, element("conn-mcp-more"), Icon::More, t!("connections.more"), None).expanded(open).on_click(cx.listener(move |this, _, _, cx| {
            let ui = &mut this.logins_ui.connections;
            ui.menu = if ui.menu.as_deref() == Some(ask_id.as_str()) { None } else { Some(ask_id.clone()) };
            ui.confirm = None;
            cx.notify();
        })));
    }
    let menu_open = mcp.server.is_some() && ui_state.menu.as_deref() == Some(row.id.as_str());
    let confirming = mcp.server.is_some() && ui_state.confirm.as_deref() == Some(row.id.as_str());
    let menu = (menu_open || confirming).then(|| {
        let n = name.clone().unwrap_or_default();
        let mut strip = h_flex().id(element("conn-mcp-menu")).w_full().px_4().pb_3().pl(px(16. + 32. + 12.)).gap_2().flex_wrap().items_center();
        if confirming {
            let yes = n.clone();
            strip = strip
                .child(div().flex_1().min_w(px(200.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.mcp_disconnect_equipe", machine = shell.endpoint.label())))
                .child(ui::button(p, element("conn-disconnect-no"), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.logins_ui.connections.confirm = None;
                    cx.notify();
                })))
                .child(ui::button(p, element("conn-disconnect-yes"), Kind::Danger, t!("connections.disconnect")).on_click(cx.listener(move |this, _, _, cx| this.connections_remove_mcp(&yes, cx))));
        } else {
            if matches!(action, McpAction::Connected | McpAction::Retry) {
                let again = n.clone();
                let label = if keyed { t!("connections.mcp_replace_key") } else { t!("connections.mcp_sign_in_again") };
                strip = strip.child(ui::button(p, element("conn-mcp-again"), Kind::Ghost, label).icon(if keyed { Icon::Lock } else { Icon::Globe }).on_click(cx.listener(move |this, _, _, cx| {
                    this.logins_ui.connections.menu = None;
                    this.connections_mcp_sign_in(&again, cx);
                })));
            }
            let ask_id = row.id.clone();
            strip = strip.child(ui::button(p, element("conn-disconnect"), Kind::Ghost, t!("connections.disconnect")).icon(Icon::Trash).on_click(cx.listener(move |this, _, _, cx| {
                this.logins_ui.connections.menu = None;
                this.logins_ui.connections.confirm = Some(ask_id.clone());
                cx.notify();
            })));
        }
        strip
    });
    let foot = detail.or(plan.filter(|_| mcp.server.is_none()));
    let lit = ui_state.lit.as_ref().is_some_and(|(r, until)| *r == row.id && Instant::now() < *until);
    v_flex()
        .id(element("conn-row"))
        .w_full()
        .when(divider, |b| b.border_t_1().border_color(p.hair))
        .when(lit, |b| b.bg(p.accent_bg))
        .child(
            h_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_3()
                .items_center()
                .child(mark_tile(p, row, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(row.label.clone()))
                        .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(used.unwrap_or_default()))
                        .child(h_flex().w_full().pt_1().gap_2().flex_wrap().items_center().child(chips).children(foot.map(|f| div().text_size(px(11.5)).text_color(p.muted).child(f)))),
                )
                .child(actions),
        )
        .children(menu)
        .into_any_element()
}

/// What a server's Sign in (or Add key) opens.
fn asked_for(server: &McpServer) -> Asked {
    match &server.auth {
        McpAuth::Key { secret, .. } => Asked::Key { secret: secret.clone() },
        _ => Asked::SignIn { server: server.name.clone() },
    }
}

/// The row an MCP server stands on.
fn row_of(server: &McpServer) -> String {
    squad_mcp::service_of(server).map_or_else(|| format!("mcp:{}", server.name), |s| s.id.to_owned())
}

/// The sheet over the page: the card asked for from a row — its own card,
/// in front of everything — or, until it comes, that it is coming, and if
/// it doesn't, why.
pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let sheet = shell.logins_ui.connections.sheet.clone()?;
    let card = match &sheet.what {
        Asked::Key { secret } => shell.model.secret_asks.iter().find(|a| a.store.as_ref().is_some_and(|st| st.name == *secret)).and_then(|a| super::secrets::card(shell, a.id, cx)),
        Asked::SignIn { server } => card_for(shell, server).and_then(|id| super::cli_signin::card(shell, id, cx)),
    };
    let body = match card {
        Some(card) => card,
        None => {
            let late = sheet.at.elapsed() > ANSWER_WAIT;
            let words = match (&sheet.what, late) {
                (_, true) => t!("connections.sheet_no_answer", machine = shell.endpoint.label()).to_string(),
                (Asked::Key { .. }, false) => t!("connections.sheet_opening_key").to_string(),
                (Asked::SignIn { .. }, false) => t!("connections.sheet_opening_signin").to_string(),
            };
            v_flex()
                .gap_3()
                .child(div().id("connections-sheet-wait").text_size(px(14.)).line_height(px(20.)).text_color(if late { p.danger() } else { p.text2 }).child(words))
                .child(h_flex().justify_end().child(ui::button(p, "connections-sheet-close", Kind::Ghost, t!("connections.sheet_close")).on_click(cx.listener(|this, _, _, cx| this.connections_hide(cx)))))
                .into_any_element()
        }
    };
    Some(super::connect::modal_w(p, "connections-sheet", 520., v_flex().id("connections-sheet-body").child(body)))
}

/// The sign-in card for server `name`, if one is up.
fn card_for(shell: &Shell, name: &str) -> Option<u64> {
    shell.model.cli_sign_ins.iter().find(|c| super::cli_signin::is_mcp(c) && (c.server.as_deref() == Some(name) || (c.server.is_none() && c.why == name))).map(|c| c.id)
}

impl Shell {
    /// Connect on an MCP service's row: its official server added for the
    /// catalog's roles, and its sign-in (or key) card as the sheet.
    fn connections_connect_mcp(&mut self, service: &str, cx: &mut Context<Self>) {
        let Ok(server) = squad_mcp::server(service, None, None, None) else { return };
        let (row, asked) = (row_of(&server), asked_for(&server));
        let squad = self.model.squad.clone();
        self.send_command(Command::AddMcpServer { squad, server, sign_in: true });
        self.connections_show(&row, asked, cx);
    }

    /// Sign in, Add key, Sign in again, Replace key: the card, as the sheet.
    fn connections_mcp_sign_in(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(server) = self.model.mcp_servers.iter().find(|s| s.name == name).cloned() else { return };
        let squad = self.model.squad.clone();
        self.send_command(Command::SignInMcpServer { squad, name: name.to_owned() });
        self.connections_show(&row_of(&server), asked_for(&server), cx);
    }

    /// Put `asked`'s card up as the sheet for `row`; until it comes the sheet
    /// says it is coming, and after [`ANSWER_WAIT`] why it didn't.
    pub fn connections_show(&mut self, row: &str, asked: Asked, cx: &mut Context<Self>) {
        let was_connected = self.model.mcp_status.iter().any(|st| self.model.mcp_servers.iter().any(|s| s.name == st.name && row_of(s) == row) && st.overall() == McpState::Connected);
        self.logins_ui.connections.sheet = Some(Sheet { row: row.to_owned(), what: asked, at: Instant::now(), seen: false, was_connected });
        self.logins_ui.connections.menu = None;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ANSWER_WAIT + Duration::from_millis(100)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        cx.notify();
    }

    /// Esc, a press beside the sheet, Close: the sheet goes (a sign-in under
    /// way goes on; Sign in on the row brings its card back).
    pub fn connections_hide(&mut self, cx: &mut Context<Self>) {
        self.logins_ui.connections.sheet = None;
        cx.notify();
    }

    /// The sheet follows its card: the box gets the keyboard when the card
    /// comes; signed in, saved, or connected, the sheet closes and its row
    /// lights up; put off, it just closes.
    pub fn connections_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.logins_ui.connections.sheet.clone() else { return };
        let connected = self.model.mcp_status.iter().any(|st| self.model.mcp_servers.iter().any(|s| s.name == st.name && row_of(s) == sheet.row) && st.overall() == McpState::Connected);
        let (present, done) = match &sheet.what {
            Asked::Key { secret } => {
                let ask = self.model.secret_asks.iter().find(|a| a.store.as_ref().is_some_and(|st| st.name == *secret)).map(|a| a.id);
                let saved = self.secrets.saved.iter().any(|(name, _)| name == secret);
                if let (Some(id), false) = (ask, sheet.seen) {
                    self.open_secret_card(id, window, cx);
                }
                (ask.is_some(), saved)
            }
            Asked::SignIn { server } => {
                let card = card_for(self, server).and_then(|id| self.model.cli_sign_ins.iter().find(|c| c.id == id).cloned());
                if let (Some(card), false) = (&card, sheet.seen) {
                    self.cli_signin_focus(card.id, window, cx);
                }
                (card.is_some(), card.is_some_and(|c| c.phase == equipe_core::protocol::cli_signin::Phase::SignedIn))
            }
        };
        let finished = done || (connected && !sheet.was_connected);
        if finished || (sheet.seen && !present) {
            self.logins_ui.connections.sheet = None;
            if finished {
                self.logins_ui.connections.lit = Some((sheet.row.clone(), Instant::now() + LIT_FOR));
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(LIT_FOR + Duration::from_millis(50)).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
            cx.notify();
        } else if present && !sheet.seen
            && let Some(up) = self.logins_ui.connections.sheet.as_mut()
        {
            up.seen = true;
        }
    }

    fn connections_remove_mcp(&mut self, name: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::RemoveMcpServer { squad, name: name.to_owned() });
        self.logins_ui.connections.confirm = None;
        self.logins_ui.connections.menu = None;
        cx.notify();
    }

    /// "Add": the server typed in, checked here as the daemon will, then
    /// added and signed in to; what is wrong is said under the boxes.
    fn connections_add_mcp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.logins_ui.connections;
        let (name, url, roles) = (ui.mcp_name.read(cx).value().to_string(), ui.mcp_url.read(cx).value().to_string(), ui.mcp_roles.read(cx).value().to_string());
        let roles: Vec<String> = roles.split(',').map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned).collect();
        match squad_mcp::server(&name, Some(&url), Some(&roles), None) {
            Ok(server) => {
                let squad = self.model.squad.clone();
                self.send_command(Command::AddMcpServer { squad, server, sign_in: true });
                let ui = &self.logins_ui.connections;
                for input in [ui.mcp_name.clone(), ui.mcp_url.clone(), ui.mcp_roles.clone()] {
                    input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                }
                self.logins_ui.connections.mcp_problem = None;
            }
            Err(why) => self.logins_ui.connections.mcp_problem = Some(why),
        }
        cx.notify();
    }

    /// Connect: the key card for the catalog service — the same card a
    /// member's `request_secret` puts up — as the sheet.
    fn connections_connect(&mut self, service: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::RequestServiceKey { squad, service: service.to_owned() });
        if let Some(secret) = Service::by_id(service).and_then(|s| s.key_name) {
            self.connections_show(service, Asked::Key { secret: secret.to_owned() }, cx);
        }
        cx.notify();
    }

    /// Disconnect: every key this squad kept for the service is deleted from the secret store.
    fn connections_disconnect(&mut self, id: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        let names: Vec<String> = rows(self).into_iter().filter(|r| r.id == id).flat_map(|r| r.keys).collect();
        for name in names {
            self.send_command(Command::DeleteSquadSecret { squad: squad.clone(), name });
        }
        self.logins_ui.connections.confirm = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob brings its own `test` macro over `#[test]`.
    use super::{CATALOG, add_mcp_rows, connections, rows_of};
    use equipe_core::protocol::Engine;
    use equipe_core::protocol::mcp_servers::{McpEngineStatus, McpServerStatus, McpState};
    use equipe_core::protocol::secrets::{SecretKind, SecretUse, SquadSecret};

    fn key(squad: &str, name: &str, service: &str, used: Option<u64>) -> SquadSecret {
        SquadSecret {
            squad: squad.into(),
            name: name.into(),
            kind: SecretKind::Token,
            service: service.into(),
            projects: Vec::new(),
            any_project: false,
            created_by: "pm".into(),
            created_ticket: None,
            created_at: 1,
            note: String::new(),
            last_used: used.map(|at| SecretUse { at, member: "backend".into(), ticket: None, project: String::new(), how: "env".into() }),
        }
    }

    fn has_words(language: &str, id: &str, part: &str) -> bool {
        let text = crate::i18n::FILES.iter().find(|(code, _)| *code == language).map(|(_, text)| *text).unwrap_or_default();
        let words: std::collections::HashMap<String, String> = serde_json::from_str(text).unwrap_or_default();
        words.contains_key(&format!("connections.svc.{id}.{part}"))
    }

    #[test]
    fn every_catalog_service_has_a_row_with_the_words_it_shows() {
        let rows = rows_of("acme", &[], &[]);
        assert_eq!(rows.len(), CATALOG.len());
        for row in &rows {
            assert!(row.keys.is_empty());
            assert!(row.status.last_used.is_none());
            // What it is for, and what to create: said in every language the app speaks.
            for (language, _) in crate::i18n::FILES {
                assert!(has_words(language, &row.id, "use"), "{language}: {} has no use", row.id);
                if row.service.is_some_and(|s| s.takes_key()) {
                    assert!(has_words(language, &row.id, "hint"), "{language}: {} has no hint", row.id);
                }
            }
        }
    }

    #[test]
    fn a_kept_key_shows_on_its_service_and_a_service_outside_the_catalog_gets_a_row_of_its_own() {
        let secrets = vec![key("acme", "anthropic-admin-key", "anthropic", Some(90)), key("acme", "acme-a", "deploy.acme.example", Some(10)), key("acme", "acme-b", "deploy.acme.example", Some(70)), key("other", "theirs", "github", None)];
        let rows = rows_of("acme", &secrets, &[]);
        let anthropic = rows.iter().find(|r| r.id == "anthropic").unwrap();
        assert_eq!((anthropic.keys.as_slice(), anthropic.status.last_used.as_ref().map(|u| u.at)), (["anthropic-admin-key".to_owned()].as_slice(), Some(90)));
        assert!(rows.iter().find(|r| r.id == "github").unwrap().keys.is_empty(), "another squad's key is not this squad's");
        let own = rows.iter().find(|r| r.id == "deploy.acme.example").unwrap();
        assert!(own.service.is_none());
        assert_eq!((own.keys.clone(), own.status.last_used.as_ref().map(|u| u.at)), (vec!["acme-a".to_owned(), "acme-b".to_owned()], Some(70)), "one row for the service, the latest use");
        // Disconnect takes away exactly the keys of the row.
        assert_eq!(rows.len(), CATALOG.len() + 1);
    }

    #[test]
    fn mobbin_has_its_row_with_its_server_and_any_other_server_a_row_of_its_own() {
        let mut rows = rows_of("acme", &[], &[]);
        add_mcp_rows(&mut rows, &[], &[]);
        let mobbin = rows.iter().find(|r| r.id == "mobbin").unwrap();
        assert!(mobbin.mcp.as_ref().is_some_and(|m| m.server.is_none()), "not connected yet: Connect");
        assert!(rows.iter().filter(|r| !["mobbin", "21st", "figma", "framer"].contains(&r.id.as_str())).all(|r| r.mcp.is_none()));
        assert!(rows.iter().any(|r| r.id == "21st" && r.mcp.as_ref().is_some_and(|m| m.server.is_none())), "21st.dev too: Connect");
        let servers = vec![equipe_core::squad_mcp::server("mobbin", None, None, None).unwrap(), equipe_core::squad_mcp::server("docs", Some("https://docs.example.com/mcp"), None, None).unwrap()];
        let status = vec![McpServerStatus { name: "mobbin".into(), engines: vec![McpEngineStatus { engine: Engine::Claude, state: McpState::NeedsSignIn, detail: None, checked_at: 1 }] }];
        let mut rows = rows_of("acme", &[], &[]);
        add_mcp_rows(&mut rows, &servers, &status);
        let mobbin = rows.iter().find(|r| r.id == "mobbin").unwrap().mcp.clone().unwrap();
        assert_eq!((mobbin.server.map(|s| s.name), mobbin.status.map(|s| s.overall())), (Some("mobbin".to_owned()), Some(McpState::NeedsSignIn)));
        let docs = rows.last().unwrap();
        assert_eq!((docs.id.as_str(), docs.label.as_str()), ("mcp:docs", "docs"));
        assert_eq!(rows.len(), CATALOG.len() + 1);
    }

    #[test]
    fn a_server_added_by_the_figma_or_framer_address_is_on_their_catalog_row() {
        use equipe_core::squad_mcp::{catalog_service, server};
        let figma = server("design-files", Some("https://mcp.figma.com/mcp"), None, None).unwrap();
        let framer = server("sites", Some("https://mcp.framer.com/mcp/"), None, None).unwrap();
        assert!(figma.catalog.is_none() && framer.catalog.is_none(), "added by address, not from the catalog");
        assert_eq!((catalog_service(&figma).map(|s| s.id), catalog_service(&framer).map(|s| s.id)), (Some("figma"), Some("framer")));
        assert!(catalog_service(&server("docs", Some("https://docs.example.com/mcp"), None, None).unwrap()).is_none());
        // Another host of the same brand is not its official server.
        assert!(catalog_service(&server("figma-docs", Some("https://docs.figma.com/mcp"), None, None).unwrap()).is_none());
        let other = server("docs", Some("https://docs.example.com/mcp"), None, None).unwrap();
        let mut rows = rows_of("acme", &[], &[]);
        add_mcp_rows(&mut rows, &[figma, framer, other], &[]);
        assert_eq!(rows.len(), CATALOG.len() + 1);
        let on = |id: &str| rows.iter().find(|r| r.id == id).unwrap().mcp.as_ref().and_then(|m| m.server.as_ref()).map(|s| s.name.clone());
        assert_eq!((on("figma"), on("framer")), (Some("design-files".to_owned()), Some("sites".to_owned())));
        assert_eq!(rows.last().unwrap().id, "mcp:docs");
    }

    #[test]
    fn each_state_has_one_obvious_action() {
        use super::{McpAction, McpRowState, mcp_action};
        let row = |server: Option<(bool, bool)>, state: Option<McpState>| mcp_action(&McpRowState { server, state });
        assert_eq!(row(None, None), McpAction::Connect, "not connected: Connect");
        assert_eq!(row(Some((false, true)), None), McpAction::Quiet, "being looked at");
        assert_eq!(row(Some((false, true)), Some(McpState::NeedsSignIn)), McpAction::SignIn);
        assert_eq!(row(Some((true, true)), Some(McpState::NeedsKey)), McpAction::AddKey, "a keyed server: its key, never a sign-in");
        assert_eq!(row(Some((true, true)), Some(McpState::NeedsSignIn)), McpAction::AddKey, "even from an older daemon");
        assert_eq!(row(Some((false, true)), Some(McpState::Connected)), McpAction::Connected);
        assert_eq!(row(Some((true, true)), Some(McpState::Connected)), McpAction::Connected);
        assert_eq!(row(Some((false, true)), Some(McpState::Failing)), McpAction::Retry);
        assert_eq!(row(Some((false, false)), Some(McpState::NeedsSignIn)), McpAction::Quiet, "off: nothing to press");
    }

    #[test]
    fn sign_in_and_add_key_open_the_right_card_for_the_right_row() {
        use super::{Asked, asked_for, row_of};
        let mobbin = equipe_core::squad_mcp::server("mobbin", None, None, None).unwrap();
        let ui = equipe_core::squad_mcp::server("21st", None, None, None).unwrap();
        let own = equipe_core::squad_mcp::server("docs", Some("https://docs.example.com/mcp"), None, None).unwrap();
        assert_eq!((row_of(&mobbin), asked_for(&mobbin)), ("mobbin".to_owned(), Asked::SignIn { server: "mobbin".into() }));
        assert_eq!((row_of(&ui), asked_for(&ui)), ("21st".to_owned(), Asked::Key { secret: "21st-api-key".into() }), "a keyed server: its key card, never a sign-in");
        assert_eq!((row_of(&own), asked_for(&own)), ("mcp:docs".to_owned(), Asked::SignIn { server: "docs".into() }));
    }

    #[test]
    fn a_card_opens_only_an_address_that_checks_out() {
        // The card re-checks the address a daemon sent before opening it on this screen.
        for good in ["https://platform.claude.com/settings/admin-keys", "https://vercel.com/account/tokens"] {
            assert!(connections::clean_url(good).is_ok(), "{good}");
        }
        for bad in ["http://platform.claude.com/", "javascript:alert(1)", "https://user@platform.claude.com/", "file:///etc/hosts"] {
            assert!(connections::clean_url(bad).is_err(), "{bad}");
        }
    }
}
