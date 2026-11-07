//! A call — the voice radio (`docs/remote-protocol.md`, "A call").
//!
//! [`Call`] is the loop itself, without I/O: the daemon's runtime and
//! `equipe call-sim` both drive it and carry out the [`Effect`]s it returns.
//! An effect is only ever a submit to a member or an event for the screens:
//! nothing said in a call decides, accepts, approves or merges anything.

use std::collections::VecDeque;

use equipe_protocol::call::{CallEnded, CallRoute, CallTarget, Dispatched, RouteBy, SayKind, TargetKind};
use equipe_protocol::{Event, SessionId};

pub mod judge;

use judge::{Intent, JudgeInput, Judged, RosterMember};

/// Turns kept whole; older ones are folded into the summary.
pub const TURNS: usize = 8;
/// The summary's most bytes: older words drop off its front.
pub const SUMMARY_MAX: usize = 800;
/// Silence before "Anything else?" (ms).
pub const IDLE_ASK_MS: i64 = 2 * 60 * 1000;
/// Silence after that before the call ends (ms).
pub const IDLE_END_MS: i64 = 60 * 1000;

/// Who said a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Person,
    Equipe,
}

/// One line of the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub who: Who,
    pub text: String,
}

/// What the loop asks its driver to do.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Send `text` to the member as an ordinary submit (from the phone).
    Submit { session: SessionId, text: String },
    /// Tell the screens.
    Publish(Event),
    /// Judge this off the loop, then hand the answer to [`Call::judged`].
    Judge(JudgeInput),
}

/// A fresh call id.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    format!("call-{:x}{:x}", nanos & 0xffff_ffff_ffff, NEXT.fetch_add(1, Ordering::Relaxed))
}

/// One call's state.
#[derive(Debug, Clone)]
pub struct Call {
    pub id: String,
    pub squad: String,
    /// The last [`TURNS`] turns.
    pub turns: VecDeque<Turn>,
    /// Older turns, folded.
    pub summary: String,
    pub dispatched: Vec<Dispatched>,
    /// The utterance being judged.
    pub pending: Option<String>,
    /// Unix ms the person was last heard (or the call began).
    pub last_heard: i64,
    roster: Vec<RosterMember>,
    /// When "Anything else?" was said for the silence, if it was.
    idle_asked: Option<i64>,
    /// Already asked once who an utterance was for.
    asked: bool,
    ended: bool,
}

impl Call {
    pub fn new(id: String, squad: String, now_ms: i64) -> (Call, Vec<Effect>) {
        let started = Event::CallStarted { squad: squad.clone(), call: id.clone() };
        let call = Call {
            id,
            squad,
            turns: VecDeque::new(),
            summary: String::new(),
            dispatched: Vec::new(),
            pending: None,
            last_heard: now_ms,
            roster: Vec::new(),
            idle_asked: None,
            asked: false,
            ended: false,
        };
        (call, vec![Effect::Publish(started)])
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    /// The person said `text`: it goes to the judge.
    pub fn hear(&mut self, text: &str, roster: Vec<RosterMember>, now_ms: i64) -> Vec<Effect> {
        let text = text.trim();
        if self.ended || text.is_empty() {
            return Vec::new();
        }
        self.last_heard = now_ms;
        self.idle_asked = None;
        self.roster = roster;
        // What the judge reads holds the turns before this one; the utterance comes on its own.
        let input = JudgeInput {
            summary: self.summary.clone(),
            turns: self.turns.iter().cloned().collect(),
            // The last few sent only, so the judge's input stays flat; the summary event has them all.
            dispatched: self.dispatched[self.dispatched.len().saturating_sub(TURNS)..].to_vec(),
            pending: self.pending.clone(),
            utterance: text.to_owned(),
            roster: self.roster.clone(),
        };
        self.push(Who::Person, text);
        self.pending = Some(text.to_owned());
        vec![Effect::Judge(input)]
    }

    /// The judge's answer for the pending utterance.
    pub fn judged(&mut self, mut j: Judged, now_ms: i64) -> Vec<Effect> {
        if self.ended {
            return Vec::new();
        }
        let said = self.pending.take().unwrap_or_default();
        let mut out = Vec::new();
        if j.intent == Intent::End {
            out.extend(self.end(CallEnded::Person));
            return out;
        }
        if j.routes.is_empty() {
            if !self.asked {
                // Asked once only; the next one that can't be told goes to the PM.
                self.asked = true;
                let text = j.reply.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| "Sorry, who is that for?".to_owned());
                out.push(self.say(&text, SayKind::Ask));
                return out;
            }
            j.routes = vec![CallRoute { to: pm_target(&self.roster), text: said.clone() }];
            j.by = RouteBy::Fallback;
        }
        self.asked = false;
        for route in &j.routes {
            for session in self.sessions_of(&route.to) {
                out.push(Effect::Submit { session, text: format!("[The person, by voice on a call] {}", route.text) });
            }
            self.dispatched.push(Dispatched { to: route.to.clone(), text: route.text.clone(), at: now_ms });
        }
        let reply = j.reply.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| sent_to(&j.routes));
        out.push(Effect::Publish(Event::CallRoute { squad: self.squad.clone(), call: self.id.clone(), routes: j.routes, by: j.by }));
        out.push(self.say(&reply, SayKind::Reply));
        out
    }

    /// The clock: silence asks "Anything else?", then ends the call.
    pub fn tick(&mut self, now_ms: i64) -> Vec<Effect> {
        if self.ended {
            return Vec::new();
        }
        match self.idle_asked {
            None if now_ms - self.last_heard >= IDLE_ASK_MS => {
                self.idle_asked = Some(now_ms);
                vec![self.say("Anything else?", SayKind::Ask)]
            }
            Some(asked) if now_ms - asked >= IDLE_END_MS => self.end(CallEnded::Idle),
            _ => Vec::new(),
        }
    }

    /// The person hung up.
    pub fn hang_up(&mut self) -> Vec<Effect> {
        if self.ended {
            return Vec::new();
        }
        self.ended = true;
        vec![self.summary_event(CallEnded::Hangup)]
    }

    fn end(&mut self, why: CallEnded) -> Vec<Effect> {
        let bye = self.say("Okay, bye.", SayKind::Bye);
        self.ended = true;
        vec![bye, self.summary_event(why)]
    }

    fn summary_event(&self, ended: CallEnded) -> Effect {
        Effect::Publish(Event::CallSummary { squad: self.squad.clone(), call: self.id.clone(), dispatched: self.dispatched.clone(), ended })
    }

    fn say(&mut self, text: &str, kind: SayKind) -> Effect {
        self.push(Who::Equipe, text);
        Effect::Publish(Event::CallSay { squad: self.squad.clone(), call: self.id.clone(), text: text.to_owned(), kind })
    }

    /// Keep the last [`TURNS`]; fold the rest into the summary.
    fn push(&mut self, who: Who, text: &str) {
        self.turns.push_back(Turn { who, text: text.to_owned() });
        while self.turns.len() > TURNS {
            let Some(old) = self.turns.pop_front() else { break };
            fold(&mut self.summary, &old);
        }
    }

    /// The sessions a target reaches: everyone, for the squad.
    fn sessions_of(&self, to: &CallTarget) -> Vec<SessionId> {
        match to.kind {
            TargetKind::Squad => self.roster.iter().map(|m| m.session).collect(),
            TargetKind::Member | TargetKind::Pm => to.session.into_iter().collect(),
        }
    }
}

/// Fold one turn into the summary, dropping the oldest words past [`SUMMARY_MAX`].
pub fn fold(summary: &mut String, turn: &Turn) {
    let who = match turn.who {
        Who::Person => "Person",
        Who::Equipe => "equipe",
    };
    if !summary.is_empty() {
        summary.push(' ');
    }
    summary.push_str(&format!("{who}: {}", turn.text.trim()));
    if summary.len() > SUMMARY_MAX {
        let mut cut = summary.len() - SUMMARY_MAX;
        while !summary.is_char_boundary(cut) {
            cut += 1;
        }
        // Start at a word.
        let rest = &summary[cut..];
        let rest = rest.find(' ').map_or(rest, |i| &rest[i + 1..]);
        *summary = format!("… {rest}");
    }
}

/// The squad's PM as a target (no session when there is none).
pub fn pm_target(roster: &[RosterMember]) -> CallTarget {
    match roster.iter().find(|m| m.role == "pm") {
        Some(pm) => CallTarget { kind: TargetKind::Pm, session: Some(pm.session), name: Some(pm.name.clone()), role: Some(pm.role.clone()) },
        None => CallTarget { kind: TargetKind::Pm, session: None, name: None, role: Some("pm".into()) },
    }
}

/// "Sent to Designer and Backend. Anything else?"
fn sent_to(routes: &[CallRoute]) -> String {
    let mut names: Vec<String> = Vec::new();
    for r in routes {
        let name = match r.to.kind {
            TargetKind::Squad => "everyone".to_owned(),
            TargetKind::Pm => "the PM".to_owned(),
            TargetKind::Member => title(r.to.name.as_deref().or(r.to.role.as_deref()).unwrap_or("them")),
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let list = match names.len() {
        0 => "nobody".to_owned(),
        1 => names[0].clone(),
        n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
    };
    format!("Sent to {list}. Anything else?")
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Vec<RosterMember> {
        [("pm", 1), ("designer", 2), ("backend", 3)].iter().map(|&(r, s)| RosterMember { session: s, name: r.into(), role: r.into(), tickets: vec![] }).collect()
    }

    fn member(r: &RosterMember) -> CallTarget {
        CallTarget { kind: TargetKind::Member, session: Some(r.session), name: Some(r.name.clone()), role: Some(r.role.clone()) }
    }

    fn judged(intent: Intent, routes: Vec<CallRoute>) -> Judged {
        Judged { intent, routes, reply: None, by: RouteBy::Llm, millis: 1, input_bytes: 0 }
    }

    fn events(effects: &[Effect]) -> Vec<&Event> {
        effects.iter().filter_map(|e| if let Effect::Publish(e) = e { Some(e) } else { None }).collect()
    }

    #[test]
    fn golden_path_routes_then_ends_with_both_in_the_summary() {
        let r = roster();
        let (mut call, started) = Call::new("c1".into(), "acme".into(), 0);
        assert!(matches!(events(&started)[..], [Event::CallStarted { .. }]));
        let fx = call.hear("Designer, redo the mockups. Backend, check the API.", r.clone(), 10);
        assert!(matches!(fx[..], [Effect::Judge(_)]));
        let routes = vec![CallRoute { to: member(&r[1]), text: "Redo the mockups.".into() }, CallRoute { to: member(&r[2]), text: "Check the API.".into() }];
        let fx = call.judged(judged(Intent::Instruct, routes), 20);
        let submits: Vec<SessionId> = fx.iter().filter_map(|e| if let Effect::Submit { session, .. } = e { Some(*session) } else { None }).collect();
        assert_eq!(submits, vec![2, 3]);
        let ev = events(&fx);
        assert!(matches!(ev[0], Event::CallRoute { routes, .. } if routes.len() == 2));
        assert!(matches!(ev[1], Event::CallSay { text, kind: SayKind::Reply, .. } if text == "Sent to Designer and Backend. Anything else?"));

        call.hear("No, that's all.", r, 30);
        let fx = call.judged(judged(Intent::End, vec![]), 40);
        let ev = events(&fx);
        assert!(matches!(ev[0], Event::CallSay { kind: SayKind::Bye, .. }));
        assert!(matches!(ev[1], Event::CallSummary { dispatched, ended: CallEnded::Person, .. } if dispatched.len() == 2));
        assert!(call.ended());
    }

    #[test]
    fn turns_stay_eight_and_the_rest_folds_into_a_bounded_summary() {
        let r = roster();
        let (mut call, _) = Call::new("c".into(), "acme".into(), 0);
        let mut sizes = Vec::new();
        for i in 0..40 {
            let fx = call.hear(&format!("Designer, do thing number {i} with some longer words to fill it"), r.clone(), i);
            let Effect::Judge(input) = &fx[0] else { panic!() };
            assert!(input.turns.len() <= TURNS);
            sizes.push(input.summary.len() + input.turns.iter().map(|t| t.text.len()).sum::<usize>());
            call.judged(judged(Intent::Instruct, vec![CallRoute { to: member(&r[1]), text: "x".into() }]), i);
        }
        assert_eq!(call.turns.len(), TURNS);
        assert!(call.summary.len() <= SUMMARY_MAX + 4, "{}", call.summary.len());
        assert!(call.summary.contains("thing number 3"));
        // Flat once the summary is full.
        assert!(sizes[39] <= sizes[20] + 16, "{sizes:?}");
    }

    #[test]
    fn asks_once_then_sends_to_the_pm() {
        let r = roster();
        let (mut call, _) = Call::new("c".into(), "acme".into(), 0);
        call.hear("Hmm.", r.clone(), 1);
        let fx = call.judged(judged(Intent::Unclear, vec![]), 2);
        assert!(matches!(events(&fx)[..], [Event::CallSay { kind: SayKind::Ask, .. }]));
        call.hear("Just do it.", r, 3);
        let fx = call.judged(judged(Intent::Unclear, vec![]), 4);
        assert!(fx.iter().any(|e| matches!(e, Effect::Submit { session: 1, .. })));
        assert!(events(&fx).iter().any(|e| matches!(e, Event::CallRoute { by: RouteBy::Fallback, .. })));
    }

    #[test]
    fn idle_asks_then_ends() {
        let (mut call, _) = Call::new("c".into(), "acme".into(), 0);
        assert!(call.tick(IDLE_ASK_MS - 1).is_empty());
        let fx = call.tick(IDLE_ASK_MS);
        assert!(matches!(events(&fx)[..], [Event::CallSay { text, .. }] if text == "Anything else?"));
        assert!(call.tick(IDLE_ASK_MS + 1000).is_empty());
        let fx = call.tick(IDLE_ASK_MS + IDLE_END_MS);
        assert!(matches!(events(&fx)[1], Event::CallSummary { ended: CallEnded::Idle, .. }));
        assert!(call.ended() && call.tick(i64::MAX / 2).is_empty());
    }

    #[test]
    fn everyone_reaches_every_member_and_hangup_summarises() {
        let r = roster();
        let (mut call, _) = Call::new("c".into(), "acme".into(), 0);
        call.hear("Everyone, stand up.", r, 1);
        let to = CallTarget { kind: TargetKind::Squad, session: None, name: None, role: None };
        let fx = call.judged(judged(Intent::Instruct, vec![CallRoute { to, text: "Stand up.".into() }]), 2);
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Submit { .. })).count(), 3);
        let fx = call.hang_up();
        assert!(matches!(events(&fx)[..], [Event::CallSummary { ended: CallEnded::Hangup, .. }]));
    }
}
