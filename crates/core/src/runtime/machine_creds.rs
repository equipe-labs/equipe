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
