//! `ask_secret` on the daemon: the request kept (in the squad's journal, so a
//! restart doesn't lose it) and shown on every screen; the person's values
//! kept in the system's secret store (login autopilot's command-line values:
//! the squad's members' commands get them as environment variables at run
//! time) and written to a file only where the member named one for a
//! program equipe doesn't run; the member told — or that the person said
//! no — and set going again.

use crate::protocol::secrets::{SecretAsk, SecretValue};

use super::*;

impl Runtime {
    /// The person's home folder: secrets are saved under it.
    pub(super) fn person_home(&self) -> Option<PathBuf> {
        self.config.person_home.clone().or_else(|| std::env::var_os("HOME").map(PathBuf::from))
    }

    /// `squad`'s secrets waiting for the person, read from its journal once.
    pub(super) fn secret_asks(&mut self, squad: &str) -> &mut Vec<SecretAsk> {
        if !self.secret_asks.contains_key(squad) {
            let kept: Vec<SecretAsk> = self.journal(squad).and_then(|j| j.secret_asks()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            self.secret_asks.insert(squad.to_owned(), kept);
        }
        self.secret_asks.get_mut(squad).expect("just put")
    }

    fn keep_secret_asks(&mut self, squad: &str) {
        let json = serde_json::to_string(self.secret_asks(squad)).unwrap_or_default();
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_secret_asks(&json);
        }
    }

    /// A member asks: checked, kept and put on a card; it carries on meanwhile.
    pub(super) fn ask_secret(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let home = self.person_home().ok_or("No home folder to save it under.")?;
        let asked = crate::secrets::check(args, &home)?;
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let keys = asked.fields.iter().map(|f| f.key.clone()).collect::<Vec<_>>().join(", ");
        let ask = SecretAsk {
            id: now_secs() * 1000 + self.secret_asks(squad).len() as u64,
            session: id,
            member: member.to_owned(),
            ticket,
            why: asked.why,
            fields: asked.fields,
            save_to: asked.shown.clone(),
            at: now_secs(),
        };
        self.secret_asks(squad).retain(|a| !(a.session == id && a.save_to == ask.save_to));
        self.secret_asks(squad).push(ask.clone());
        self.keep_secret_asks(squad);
        self.publish(Event::SecretAsked { squad: squad.to_owned(), ask });
        let kept = if asked.shown.is_empty() { "keeps what they type in this machine's secret store".to_owned() } else { format!("keeps what they type in this machine's secret store and saves it to {}", asked.shown) };
        Ok(format!(
            "The person is asked on a card for {keys}; equipe {kept} — you never see the values. When they save it you are \
told, and carry on from there. Meanwhile go on with what doesn't need it, or end your turn saying in a line what you're \
waiting for."
        ))
    }

    /// The person's values, to the file; the card closes and the member goes on.
    pub(super) async fn save_secret(&mut self, squad: &str, id: u64, values: Vec<SecretValue>) {
        let Some(ask) = self.secret_asks(squad).iter().find(|a| a.id == id).cloned() else { return };
        let Some(home) = self.person_home() else { return };
        if values.len() != ask.fields.len() || values.iter().any(|v| v.0.contains(['\n', '\r', '\0'])) {
            return self.squad_error(squad, say!(self, "error.secret_not_saved", path = ask.save_to, why = "a value can't hold a line break"));
        }
        // A file only where the member named one, for a program equipe doesn't run.
        if !ask.save_to.is_empty() {
            let written = crate::secrets::place(&ask.save_to, &home).and_then(|(path, _)| crate::secrets::write(&path, &ask.fields, &values));
            if let Err(why) = written {
                return self.squad_error(squad, say!(self, "error.secret_not_saved", path = ask.save_to, why = why));
            }
        }
        // The secret store, for the squad's members' commands (as `$KEY`).
        let (vault, owner) = (self.vault.clone(), squad.to_owned());
        let pairs: Vec<(String, String)> = ask.fields.iter().map(|f| f.key.clone()).zip(values.into_iter().map(|v| v.0)).collect();
        let kept = tokio::task::spawn_blocking(move || {
            let mut v = vault.lock().map_err(|_| "the logins are unreadable".to_owned())?;
            let mut ids = Vec::new();
            for (key, value) in pairs {
                let id = v.cli.iter().find(|c| c.kind == crate::protocol::vault::CliKind::Env && c.name == key && c.squad.as_deref() == Some(owner.as_str())).map(|c| c.id);
                let draft = crate::protocol::vault::CliLoginDraft { id, kind: crate::protocol::vault::CliKind::Env, name: key, host: String::new(), public: String::new(), squad: Some(owner.clone()), secret: Some(value.clone()) };
                ids.push((v.save_cli(draft, now_secs())?, value));
            }
            Ok::<Vec<(u32, String)>, String>(ids)
        })
        .await
        .unwrap_or_else(|_| Err("stopped".into()));
        match kept {
            // In the members' next runs at once: the turn it is woken for among them.
            Ok(ids) => self.keep_run_values(ids),
            Err(why) => return self.squad_error(squad, say!(self, "error.secret_not_saved", path = ask.save_to, why = why)),
        }
        let _ = self.internal.send(Internal::Autopilot(super::autopilot::Note::Changed(Ok(()))));
        self.secret_asks(squad).retain(|a| a.id != id);
        self.keep_secret_asks(squad);
        self.publish(Event::SecretClosed { squad: squad.to_owned(), id, saved: true });
        let keys = ask.fields.iter().map(|f| f.key.clone()).collect::<Vec<_>>().join(", ");
        let shown = if ask.save_to.is_empty() { say!(self, "notice.secret_store") } else { ask.save_to.clone() };
        self.publish(Event::Notice { undo: None, session: ask.session, text: say!(self, "notice.secret_saved", keys = keys, path = shown) });
        let vars = ask.fields.iter().map(|f| format!("${}", f.key)).collect::<Vec<_>>().join(", ");
        let file = if ask.save_to.is_empty() { String::new() } else { format!(" It is in {} too (readable only by them), for the program that reads it.", ask.save_to) };
        let note = format!("The person saved {keys}: your commands get them as {vars} in every turn that starts from now.{file}");
        if self.resume_waiting_on(squad, ask.ticket.unwrap_or_default(), &[super::decide::one_line(&ask.why)], &note).await {
            return;
        }
        let prompt = format!(
            "[equipe: the person saved {keys}. Your commands get them as {vars} (environment variables of every turn that starts from now) — \
use them from there, and never print, echo or copy a value into chat, tickets, commits or logs.{file} Carry on.]"
        );
        self.set_going(ask.session, prompt).await;
    }

    /// The person won't give it: the card closes, the member hears it.
    pub(super) async fn dismiss_secret(&mut self, squad: &str, id: u64) {
        let Some(ask) = self.secret_asks(squad).iter().find(|a| a.id == id).cloned() else { return };
        self.secret_asks(squad).retain(|a| a.id != id);
        self.keep_secret_asks(squad);
        self.publish(Event::SecretClosed { squad: squad.to_owned(), id, saved: false });
        let keys = ask.fields.iter().map(|f| f.key.clone()).collect::<Vec<_>>().join(", ");
        let note = format!("The person didn't give {keys} for now. Go on without it, or say in a line what it would let you do.");
        if self.resume_waiting_on(squad, ask.ticket.unwrap_or_default(), &[super::decide::one_line(&ask.why)], &note).await {
            return;
        }
        let prompt = format!("[equipe: the person didn't give {keys} for now. Go on without it, or say in a line what it would let you do.]");
        self.set_going(ask.session, prompt).await;
    }
}
