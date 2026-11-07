//! A call — the voice radio (`crate::call`), as the daemon runs it: each
//! utterance is judged off the loop, and what it routes reaches members only
//! as a submit sent from the phone. Nothing said in a call decides anything.

use super::*;
use crate::call::{self, Call, Effect, judge};

/// Unix milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Runtime {
    /// `call_start`: a new call in `squad`.
    pub(super) async fn call_start(&mut self, squad: String) {
        let (call, effects) = Call::new(call::new_id(), squad, now_ms());
        let id = call.id.clone();
        self.calls.insert(id.clone(), call);
        self.call_effects(&id, effects).await;
    }

    /// `call_utterance`: one finished utterance, judged off the loop.
    pub(super) async fn call_utterance(&mut self, squad: String, id: String, text: String) {
        if !self.calls.get(&id).is_some_and(|c| c.squad == squad) {
            return;
        }
        let roster = self.call_roster(&squad);
        let Some(call) = self.calls.get_mut(&id) else { return };
        let effects = call.hear(&text, roster, now_ms());
        self.call_effects(&id, effects).await;
    }

    /// `call_end`: the person hung up.
    pub(super) async fn call_end(&mut self, squad: String, id: String) {
        let Some(call) = self.calls.get_mut(&id).filter(|c| c.squad == squad) else { return };
        let effects = call.hang_up();
        self.call_effects(&id, effects).await;
        self.calls.remove(&id);
    }

    /// The judge's answer for call `id`, back on the loop.
    pub(super) async fn call_judged(&mut self, id: String, judged: judge::Judged) {
        let Some(call) = self.calls.get_mut(&id) else { return };
        let effects = call.judged(judged, now_ms());
        self.call_effects(&id, effects).await;
        if self.calls.get(&id).is_some_and(|c| c.ended()) {
            self.calls.remove(&id);
        }
    }

    /// The minute tick: calls left silent ask, then end.
    pub(super) async fn calls_tick(&mut self) {
        let now = now_ms();
        let ids: Vec<String> = self.calls.keys().cloned().collect();
        for id in ids {
            let Some(call) = self.calls.get_mut(&id) else { continue };
            let effects = call.tick(now);
            self.call_effects(&id, effects).await;
        }
        self.calls.retain(|_, c| !c.ended());
    }

    /// Who in `squad` the judge may route to: members, their role, and the
    /// titles of the open tickets each is on.
    fn call_roster(&mut self, squad: &str) -> Vec<judge::RosterMember> {
        let tickets = self.tickets(squad);
        let mut roster: Vec<(u64, judge::RosterMember)> = self
            .squad_members(squad)
            .into_iter()
            .filter_map(|id| {
                let s = self.sessions.get(&id)?;
                let m = s.member.as_ref()?;
                let on = tickets.iter().filter(|t| !t.status.is_closed() && t.assignee.as_deref() == Some(m.name.as_str())).map(|t| t.title.clone()).collect();
                Some((s.joined, judge::RosterMember { session: id, name: m.name.clone(), role: m.role.clone(), tickets: on }))
            })
            .collect();
        roster.sort_by_key(|(joined, _)| *joined);
        roster.into_iter().map(|(_, m)| m).collect()
    }

    /// What a call asks for: events published, submits sent as from the
    /// phone, judging off the loop.
    async fn call_effects(&mut self, id: &str, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.publish(event),
                // Exactly what the phone's own submit does (`Command::OnPhone`
                // around `Command::Submit`) — never read for a decision in words.
                Effect::Submit { session, text } => {
                    let was = std::mem::replace(&mut self.on_phone, true);
                    Box::pin(self.handle(Command::Submit { session, text, about: None })).await;
                    self.on_phone = was;
                }
                Effect::Judge(input) => {
                    let llm = self.call_llm.clone();
                    let internal = self.internal.clone();
                    let call = id.to_owned();
                    tokio::spawn(async move {
                        let judged = judge::judge(&input, &llm).await;
                        let _ = internal.send(Internal::CallJudged { call, judged: Box::new(judged) });
                    });
                }
            }
        }
    }
}
