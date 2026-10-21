//! `ask_secret` on the daemon: the request kept (in the squad's journal, so a
//! restart doesn't lose it) and shown on every screen; the person's values
//! written to its file (`crate::secrets::write`) and nowhere else; the member
//! told where they are — or that the person said no — and set going again.

use crate::protocol::secrets::{SecretAsk, SecretValue};

use super::*;

impl Runtime {
    /// The person's home folder: secrets are saved under it.
    fn person_home(&self) -> Option<PathBuf> {
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
        Ok(format!(
            "The person is asked on a card for {keys}; equipe saves what they type to {} — you never see the values. When they \
save it you are told, and carry on from there. Meanwhile go on with what doesn't need it, or end your turn saying in a \
line what you're waiting for.",
            asked.shown
        ))
    }

    /// The person's values, to the file; the card closes and the member goes on.
    pub(super) async fn save_secret(&mut self, squad: &str, id: u64, values: Vec<SecretValue>) {
        let Some(ask) = self.secret_asks(squad).iter().find(|a| a.id == id).cloned() else { return };
        let Some(home) = self.person_home() else { return };
        let written = crate::secrets::place(&ask.save_to, &home).and_then(|(path, _)| crate::secrets::write(&path, &ask.fields, &values));
        drop(values);
        if let Err(why) = written {
            return self.squad_error(squad, say!(self, "error.secret_not_saved", path = ask.save_to, why = why));
        }
        self.secret_asks(squad).retain(|a| a.id != id);
        self.keep_secret_asks(squad);
        self.publish(Event::SecretClosed { squad: squad.to_owned(), id, saved: true });
        let keys = ask.fields.iter().map(|f| f.key.clone()).collect::<Vec<_>>().join(", ");
        self.publish(Event::Notice { session: ask.session, text: say!(self, "notice.secret_saved", keys = keys, path = ask.save_to) });
        let prompt = format!(
            "[equipe: the person saved {keys} to {} (readable only by them). Use it from there — read the file in your code \
or `source` it in a command — and never print, echo or copy a value into chat, tickets, commits or logs. Carry on.]",
            ask.save_to
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
        let prompt = format!("[equipe: the person didn't give {keys} for now. Go on without it, or say in a line what it would let you do.]");
        self.set_going(ask.session, prompt).await;
    }
}
