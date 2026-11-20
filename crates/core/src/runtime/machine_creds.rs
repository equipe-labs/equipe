//! The machine's own credentials on the daemon (`crate::machine_creds`): read
//! now and then, told to members in their briefing, and checked before a
//! `cli_sign_in` card goes up. And per-deploy previews (`crate::preview_hosts`):
//! never a page the person is put in front of.

use crate::machine_creds as mc;

use super::*;

impl Runtime {
    /// What this machine has, by name and shape. On the person's own setup
    /// (no `person_home` given) the daemon's environment is read and the tools
    /// are asked in the background; a test's home is read for its files only.
    pub(super) fn machine_inventory(&mut self) -> mc::Inventory {
        let live = self.config.person_home.is_none();
        let Some(home) = self.person_home() else { return mc::Inventory::default() };
        let env = |name: &str| std::env::var(name).ok();
        let at = mc::Where { home: &home, env: live.then_some(&env as &mc::Env) };
        self.machine_creds.inventory(&at, live)
    }

    /// The line a member's briefing carries about them, if any.
    pub(super) fn machine_briefing(&mut self) -> Option<String> {
        mc::briefing(&self.machine_inventory())
    }

    /// `cli_sign_in` held back while a credential for that tool already works here.
    pub(super) fn machine_guard(&mut self, squad: &str, member: &str, asked: &crate::cli_signin::Asked) -> Result<(), String> {
        let inventory = self.machine_inventory();
        mc::guard(asked.kind, &inventory, asked.why_not_machine.as_deref()).inspect_err(|_| {
            crate::daemon_log!("equipe: {squad}: {member}'s sign-in card for {} was refused: a machine credential already works", asked.kind.name());
        })
    }

    /// A member asking to sign in at a site in the browser (`show_my_browser`,
    /// or a `sign_in` card on `site` when `card`) where something better
    /// exists: AWS's console while the machine has a credential that works
    /// without the person (unless `why_not_machine` names the permission it
    /// lacks), or an app the desktop tools sign in to instead of the automated
    /// browser (Slack, [`crate::native_sign_in`]) while desktop control is on
    /// for the member and the app is installed. `Some(text)`: refused, the
    /// person not asked.
    pub(super) fn sign_in_elsewhere(&mut self, call: &ToolCall, url: Option<&str>, why: &str, card: bool) -> Option<String> {
        let page = self.browser_key(self.caller_id(call)?).and_then(|(squad, key)| self.lane_page(&format!("{squad}/{key}")));
        let at = url.map(str::to_owned).or_else(|| page.map(|p| p.url)).unwrap_or_default();
        let site = vault::site_of(&at);
        if site.is_empty() {
            return None;
        }
        let signing_in = card || crate::logins::cause_of_handoff(why, Some(&at), None, true).is_shared_step();
        if mc::aws_sign_in_host(&site) && (signing_in || !site.contains("console.")) {
            let why_not = call.arguments["why_not_machine"].as_str();
            let tool = if card { crate::board::SIGN_IN } else { crate::board::SHOW_BROWSER };
            let inventory = self.machine_inventory();
            if let Err(text) = mc::browser_guard(&inventory, why_not, tool) {
                crate::daemon_log!("equipe: {}: {}'s sign-in to {site} in the browser was refused: a machine credential already works", call.squad, call.member);
                return Some(text);
            }
        }
        if signing_in {
            let map = crate::native_sign_in::mapping(&crate::native_sign_in::read_extra(&self.config.home));
            if let Some(app) = crate::native_sign_in::app_for(&site, &map) {
                let dirs = crate::native_sign_in::app_dirs(self.person_home().as_deref(), self.config.person_home.is_none());
                if crate::native_sign_in::installed(&app, &dirs) && self.desktop_allowed(call).is_ok() {
                    crate::daemon_log!("equipe: {}: {}'s sign-in to {site} in the browser was refused: the {app} app does it", call.squad, call.member);
                    return Some(crate::native_sign_in::refusal(&site, &app));
                }
            }
        }
        None
    }

    /// A call that would put the person in front of a per-deploy preview: why not.
    pub(super) fn per_deploy_refusal(&self, call: &ToolCall) -> Option<String> {
        let texts = crate::preview_hosts::texts_of(&call.name, &call.arguments);
        if texts.is_empty() {
            return None;
        }
        let extra = crate::preview_hosts::read_extra(&self.config.home);
        let host = crate::preview_hosts::first_per_deploy(texts.iter().map(String::as_str), &extra)?;
        crate::daemon_log!("equipe: {}: {}'s {} on the per-deploy preview {host} was refused", call.squad, call.member, call.name);
        Some(crate::preview_hosts::refusal(&host))
    }
}
