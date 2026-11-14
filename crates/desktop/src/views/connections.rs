//! "Connections" on the Logins page: the outside services this squad uses
//! (`equipe_core::connections::CATALOG`, plus any other service a kept key is
//! for) — for each, whether a key is saved, whether the members' browser is
//! signed in (from the saved logins), when a key was last used — and the
//! buttons that start the same flows the members use: Connect (the key card,
//! `Command::RequestServiceKey`), Sign in (the in-app "Sign in to a site"
//! flow) and Disconnect (delete the saved key; signing out is the site's own).
//!
//! The list carries facts only; a key's value never comes to this page.

use equipe_core::connections::{self, CATALOG, Service, SignedIn};
use equipe_core::protocol::Command;
use equipe_core::protocol::secrets::SquadSecret;
use equipe_core::protocol::vault::Login;
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::theme::{Palette, R_LG, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// What the section remembers between draws: the service whose Disconnect waits for its second click.
#[derive(Default)]
pub struct ConnectionsUi {
    pub confirm: Option<String>,
}

/// One row: a catalog service, or another service a kept key is for.
struct Row {
    /// The catalog id, or the service text of a key outside the catalog.
    id: String,
    label: String,
    service: Option<&'static Service>,
    status: connections::Status,
    /// The key names kept for it (for Disconnect), whichever way its service was written.
    keys: Vec<String>,
}

fn rows(shell: &Shell) -> Vec<Row> {
    rows_of(shell.model.squad.as_str(), &shell.model.squad_secrets, &shell.model.logins)
}

fn rows_of(squad: &str, secrets: &[SquadSecret], logins: &[Login]) -> Vec<Row> {
    let mut out: Vec<Row> = CATALOG
        .iter()
        .map(|s| {
            let status = connections::status(s, squad, secrets, logins);
            Row { id: s.id.to_owned(), label: s.label.to_owned(), service: Some(s), keys: status.keys.clone(), status }
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
    for (ix, row) in rows(shell).iter().enumerate() {
        body = body.child(row_view(shell, row, ix > 0, cx));
    }
    // The key cards asked for — by Connect here or by a member — right where the person is.
    let ids: Vec<u64> = shell.model.secret_asks.iter().filter(|a| a.store.is_some()).map(|a| a.id).collect();
    let cards: Vec<AnyElement> = ids
        .into_iter()
        .filter_map(|id| super::secrets::card(shell, id, cx))
        .map(|card| div().id("connections-card").w_full().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(card).into_any_element())
        .collect();
    v_flex()
        .id("logins-connections")
        .w_full()
        .gap_2()
        .child(h_flex().w_full().px_1().gap_2().items_baseline().child(div().id("connections-title").text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).role(Role::Heading).child(t!("connections.title"))))
        .child(div().w_full().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connections.subtitle", machine = shell.endpoint.label())))
        .children(super::secrets::saved_line(shell))
        .children(cards)
        .child(body)
        .into_any_element()
}

fn row_view(shell: &Shell, row: &Row, divider: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let status = &row.status;
    let id = row.id.clone();
    let element = |prefix: &str| SharedString::from(format!("{prefix}-{id}"));
    let used = row.service.and_then(|s| super::secrets::service_words(s.id, "use"));
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

impl Shell {
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
    use super::{CATALOG, connections, rows_of};
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
