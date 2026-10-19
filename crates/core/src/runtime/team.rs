//! Changes to the team the PM proposes (`propose_team_change`): kept while
//! they wait for the person — in memory only, as what made one worth it (a
//! member busy for hours, a ticket waiting) doesn't outlast a restart — and
//! shown on every screen. Approved by the person's own command, a change runs
//! exactly what their `/add` or `/close` would: [`Command::Start`] for each
//! member that joins (and [`Command::SetEffort`] when it says how hard it
//! thinks), [`Command::Close`] for the one that leaves. Nothing here starts
//! or closes a member any other way, and an agent's word never approves.

use equipe_protocol::team::{Change, TeamChange};

use super::*;
use crate::team::{self as rules, Mate};

impl Runtime {
    /// `squad`'s changes waiting for the person, oldest first.
    pub(super) fn team_changes(&mut self, squad: &str) -> &mut Vec<TeamChange> {
        self.team_changes.entry(squad.to_owned()).or_default()
    }

    /// `propose_team_change`: checked against the team and the board, kept,
    /// and shown; the PM told the person decides on a card.
    pub(super) fn propose_team_change(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM proposes changes to the team. Tell the PM what would help — who is busy, what waits — and it \
proposes it to the person."
                .into());
        }
        let tickets = self.tickets(squad);
        let team: Vec<(String, String, Option<TicketId>)> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad) && s.lane.is_none())
            .filter_map(|s| s.member.as_ref().map(|m| (m.name.clone(), m.role.clone(), s.work.as_ref().map(|w| w.ticket))))
            .collect();
        let mates: Vec<Mate<'_>> = team.iter().map(|(name, role, on)| Mate { name, role, on: *on }).collect();
        let waiting = self.team_changes.get(squad).cloned().unwrap_or_default();
        let proposal = rules::check(args, &mates, &tickets, &waiting)?;
        let asked = args.get("asked").and_then(serde_json::Value::as_bool).unwrap_or(false);
        if let Some(why) = rules::too_soon(&proposal, &mates, &tickets, now_secs(), asked) {
            return Err(why);
        }
        let reply = rules::proposed_reply(&proposal);
        let change = proposal.into_change(equipe_protocol::new_session_id(), member, id, now_secs());
        self.team_changes(squad).push(change.clone());
        self.publish(Event::TeamChangeProposed { squad: squad.to_owned(), change });
        Ok(reply)
    }

    /// Take change `id` out of what waits, if it still does.
    fn take_team_change(&mut self, squad: &str, id: u64) -> Option<TeamChange> {
        let changes = self.team_changes(squad);
        let at = changes.iter().position(|c| c.id == id)?;
        Some(changes.remove(at))
    }

    /// The person's yes: the change made as their own commands make it, the
    /// card closed everywhere, and the PM told. Decided already, elsewhere:
    /// nothing more.
    pub(super) async fn approve_team_change(&mut self, squad: &str, id: u64) {
        let Some(change) = self.take_team_change(squad, id) else { return };
        let mut members = Vec::new();
        match change.change {
            Change::Add => {
                let home = squad_home::path(&self.config.home, squad);
                let access = if roles::role_or_custom(&change.role).edits { Access::EditFiles } else { Access::ReadOnly };
                for _ in 0..change.count.max(1) {
                    let session = equipe_protocol::new_session_id();
                    let spec = SessionSpec {
                        engine: change.engine,
                        model: change.model.clone(),
                        access,
                        cwd: home.clone(),
                        role: Some(change.role.clone()),
                        squad: Some(squad.to_owned()),
                    };
                    // What `/add` sends.
                    Box::pin(self.handle(Command::Start { session, spec })).await;
                    let Some(name) = self.sessions.get(&session).and_then(|s| s.name().map(str::to_owned)) else { continue };
                    if change.effort.is_some() {
                        Box::pin(self.handle(Command::SetEffort { session, effort: change.effort.clone() })).await;
                    }
                    members.push(name);
                }
            }
            Change::Remove => {
                // What `/close` on that member sends.
                if let Some(session) = change.member.as_deref().and_then(|name| self.member_id(squad, name)) {
                    members.extend(change.member.clone());
                    Box::pin(self.handle(Command::Close { session })).await;
                }
            }
        }
        self.publish(Event::TeamChangeClosed { squad: squad.to_owned(), id, approved: true, members: members.clone() });
        let names = members.join(", ");
        let (line, shown) = match (change.change, members.is_empty()) {
            (Change::Add, false) => (
                format!("The person approved your proposal: {names} joined the team; equipe hands them ready work by the board's rules."),
                say!(self, "news.team_added", names = names),
            ),
            (Change::Add, true) => (
                format!("The person approved adding {}, but nobody could join (see the error on their screen).", change.role),
                say!(self, "news.team_not_added", role = change.role),
            ),
            (Change::Remove, false) => (
                format!("The person approved your proposal: {names} left the team; what it was on went back to the board."),
                say!(self, "news.team_removed", name = names),
            ),
            (Change::Remove, true) => return,
        };
        self.brief_pm(squad, line, Some(shown));
    }

    /// The person's no: nothing changes; the PM is told, to plan without it.
    pub(super) fn decline_team_change(&mut self, squad: &str, id: u64) {
        let Some(change) = self.take_team_change(squad, id) else { return };
        self.publish(Event::TeamChangeClosed { squad: squad.to_owned(), id, approved: false, members: Vec::new() });
        let (what_en, what) = match change.change {
            Change::Add => (format!("adding {} {}", change.count, change.role), say!(self, "news.team_what_add", role = change.role, count = change.count)),
            Change::Remove => {
                let name = change.member.clone().unwrap_or_else(|| change.role.clone());
                (format!("letting {name} go"), say!(self, "news.team_what_remove", name = name))
            }
        };
        let line = format!("The person declined your proposal ({what_en}). Plan with the team as it is; don't propose it again unless something changes.");
        let shown = say!(self, "news.team_declined", what = what);
        self.brief_pm(squad, line, Some(shown));
    }

    /// Changes `which` picks no longer wait: the member they'd let go left
    /// anyway. Nobody is told — there is nothing to decide.
    pub(super) fn drop_team_changes(&mut self, squad: &str, which: impl Fn(&TeamChange) -> bool) {
        let (gone, kept): (Vec<TeamChange>, Vec<TeamChange>) = std::mem::take(self.team_changes(squad)).into_iter().partition(|c| which(c));
        *self.team_changes(squad) = kept;
        for change in gone {
            self.publish(Event::TeamChangeClosed { squad: squad.to_owned(), id: change.id, approved: false, members: Vec::new() });
        }
    }
}
