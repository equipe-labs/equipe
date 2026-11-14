//! "Connections" on the Logins page: the outside services this squad uses
//! (`equipe_core::connections::CATALOG`, plus any other service a kept key is
//! for) — for each, whether a key is saved, whether the members' browser is
//! signed in (from the saved logins), when a key was last used — and the
//! buttons that start the same flows the members use: Connect (the key card,
//! `Command::RequestServiceKey`), Sign in (the in-app "Sign in to a site"
//! flow) and Disconnect (delete the saved key; signing out is the site's own).
//!
//! A service members use through its official MCP server (Mobbin) has its
//! own row: where the server stands on each engine (connected, needs
//! sign-in, not reachable — as the engines' own tools said), Connect (adds
//! it to the squad for the catalog's roles and starts the engines' own
//! sign-in on a card here), Sign in, and Disconnect. Any other MCP server is
//! added by its address at the end of the section.
//!
//! The list carries facts only; a key's value or a token never comes to this page.

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

use crate::icons::{Icon, icon};
use crate::theme::{CONTROL, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// What the section remembers between draws: the row whose Disconnect waits
/// for its second click, and the boxes of "Add an MCP server".
pub struct ConnectionsUi {
    pub confirm: Option<String>,
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

/// The squad's MCP servers on their rows: a catalog one on its service's
/// row, any other on a row of its own after the catalog's.
fn add_mcp_rows(rows: &mut Vec<Row>, servers: &[McpServer], status: &[McpServerStatus]) {
    let status_of = |name: &str| status.iter().find(|s| s.name == name).cloned();
    for row in rows.iter_mut().filter(|r| r.service.is_some_and(Service::is_mcp)) {
        let server = servers.iter().find(|s| s.catalog.as_deref() == Some(row.id.as_str())).cloned();
        let status = server.as_ref().and_then(|s| status_of(&s.name));
        row.mcp = Some(McpRow { server, status });
    }
    for server in servers.iter().filter(|s| squad_mcp::service_of(s).is_none()) {
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
    // The key cards asked for — by Connect here or by a member — right where the person is.
    let ids: Vec<u64> = shell.model.secret_asks.iter().filter(|a| a.store.is_some()).map(|a| a.id).collect();
    let mut cards: Vec<AnyElement> = ids
        .into_iter()
        .filter_map(|id| super::secrets::card(shell, id, cx))
        .map(|card| div().id("connections-card").w_full().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(card).into_any_element())
        .collect();
    // An MCP server's sign-in, started here or for a member: its card here too.
    let signins: Vec<u64> = shell.model.cli_sign_ins.iter().filter(|c| c.tool.ends_with("_mcp")).map(|c| c.id).collect();
    cards.extend(
        signins
            .into_iter()
            .filter_map(|id| super::cli_signin::card(shell, id, cx).map(|card| (id, card)))
            .map(|(id, card)| div().id(("connections-mcp-card", id)).w_full().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(card).into_any_element()),
    );
    v_flex()
        .id("logins-connections")
        .w_full()
        .gap_2()
        .child(h_flex().w_full().px_1().gap_2().items_baseline().child(div().id("connections-title").text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).role(Role::Heading).child(t!("connections.title"))))
        .child(div().w_full().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.subtitle", machine = shell.endpoint.label())))
        .children(super::secrets::saved_line(shell))
        .children(cards)
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
    let used = row.service.and_then(|s| super::secrets::service_words(s.id, "use")).or_else(|| row.mcp.as_ref().and_then(|m| m.server.as_ref()).map(|s| s.url.clone()));
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
                .child(div().flex_none().size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).bg(p.wash).child(icon(Icon::Plug, 14., p.text2)))
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

/// An MCP server's row: where it stands, for which roles, and its buttons.
fn mcp_row_view(shell: &Shell, row: &Row, mcp: &McpRow, used: Option<String>, divider: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = row.id.clone();
    let element = |prefix: &str| SharedString::from(format!("{prefix}-{id}"));
    let (text, tone) = mcp_chip(p, mcp);
    let mut chips = h_flex().flex_none().gap_2().items_center().child(chip(p, text, tone));
    let roles = match (&mcp.server, row.service.and_then(|s| s.mcp)) {
        (Some(s), _) if s.roles.is_empty() => Some(t!("connections.mcp_for_everyone").to_owned()),
        (Some(s), _) => Some(t!("connections.mcp_for", roles = s.roles.join(", "))),
        (None, Some(m)) => Some(t!("connections.mcp_for", roles = m.roles.join(", "))),
        (None, None) => None,
    };
    if let Some(roles) = roles {
        chips = chips.child(chip(p, roles, p.muted));
    }
    // What its plan needs, from the catalog's words.
    let plan = row.service.and_then(|s| super::secrets::service_words(s.id, "hint"));
    let detail = mcp.status.as_ref().filter(|s| s.overall() == McpState::Failing).and_then(|s| s.detail().map(str::to_owned));
    let mut actions = h_flex().flex_none().gap_2().items_center();
    match &mcp.server {
        None => {
            if let Some(service) = row.service {
                let sid = service.id.to_owned();
                actions = actions.child(ui::button(p, element("conn-mcp-connect"), Kind::Primary, t!("connections.connect")).icon(Icon::Plug).on_click(cx.listener(move |this, _, _, cx| this.connections_connect_mcp(&sid, cx))));
                if let Some(m) = service.mcp {
                    let plans = m.sign_up_url.to_owned();
                    actions = actions.child(ui::button(p, element("conn-mcp-plans"), Kind::Ghost, t!("connections.mcp_plans")).icon(Icon::External).on_click(cx.listener(move |_, _, _, cx| cx.open_url(&plans))));
                }
            }
        }
        Some(server) => {
            let name = server.name.clone();
            let connected = mcp.status.as_ref().is_some_and(|s| s.overall() == McpState::Connected);
            if server.enabled && !connected {
                let n = name.clone();
                // A keyed server's "sign in" is its key card.
                let (label, glyph) = if matches!(server.auth, McpAuth::Key { .. }) { (t!("connections.mcp_add_key"), Icon::Lock) } else { (t!("connections.sign_in"), Icon::Globe) };
                actions = actions.child(ui::button(p, element("conn-mcp-signin"), Kind::Primary, label).icon(glyph).on_click(cx.listener(move |this, _, _, cx| this.connections_mcp_sign_in(&n, cx))));
            }
            if shell.logins_ui.connections.confirm.as_deref() == Some(row.id.as_str()) {
                actions = actions
                    .child(ui::button(p, element("conn-disconnect-no"), Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.logins_ui.connections.confirm = None;
                        cx.notify();
                    })))
                    .child(ui::button(p, element("conn-disconnect-yes"), Kind::Danger, t!("connections.disconnect_yes")).on_click(cx.listener(move |this, _, _, cx| this.connections_remove_mcp(&name, cx))));
            } else {
                let ask_id = row.id.clone();
                actions = actions.child(ui::button(p, element("conn-disconnect"), Kind::Ghost, t!("connections.disconnect")).icon(Icon::Trash).on_click(cx.listener(move |this, _, _, cx| {
                    this.logins_ui.connections.confirm = Some(ask_id.clone());
                    cx.notify();
                })));
            }
        }
    }
    let confirming = mcp.server.is_some() && shell.logins_ui.connections.confirm.as_deref() == Some(row.id.as_str());
    let foot = detail.or(plan.filter(|_| mcp.server.is_none()));
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
                .child(div().flex_none().size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).bg(p.wash).child(icon(Icon::Server, 14., p.text2)))
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
        .children(confirming.then(|| div().w_full().px_4().pb_3().pl(px(16. + 32. + 12.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.mcp_disconnect_note"))))
        .into_any_element()
}

impl Shell {
    /// Connect on an MCP service's row: its official server added for the
    /// catalog's roles, and the engines' own sign-in started on a card here.
    fn connections_connect_mcp(&mut self, service: &str, cx: &mut Context<Self>) {
        let Ok(server) = squad_mcp::server(service, None, None, None) else { return };
        let squad = self.model.squad.clone();
        self.send_command(Command::AddMcpServer { squad, server, sign_in: true });
        cx.notify();
    }

    fn connections_mcp_sign_in(&mut self, name: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::SignInMcpServer { squad, name: name.to_owned() });
        cx.notify();
    }

    fn connections_remove_mcp(&mut self, name: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::RemoveMcpServer { squad, name: name.to_owned() });
        self.logins_ui.connections.confirm = None;
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

    /// Connect: the key card for the catalog service, on every screen — the
    /// same card a member's `request_secret` puts there.
    fn connections_connect(&mut self, service: &str, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::RequestServiceKey { squad, service: service.to_owned() });
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
        assert!(rows.iter().filter(|r| !["mobbin", "21st"].contains(&r.id.as_str())).all(|r| r.mcp.is_none()));
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
