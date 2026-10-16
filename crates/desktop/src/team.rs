//! A change to the team the PM proposed (`propose_team_change`), as the card
//! the person approves it on — above the composer in the PM's conversation,
//! and in 나에게 under "결정 필요": which changes wait, and what the card says
//! of one (what it does, what waits for it and who is busy, the details, the
//! words on its button). No UI types here, so the rules are tested without a
//! window. The approval is the person's command; the daemon then does what
//! `/add` or `/close` does.

use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::protocol::{Command, Engine, Ticket, TicketId};
use equipe_core::roles;
use equipe_core::t;

// ── the changes waiting ──

/// Changes waiting for the person, oldest first. One decided here hides at
/// once, so a second click can't send it twice; the daemon's
/// `TeamChangeClosed` takes it for good.
#[derive(Debug, Clone, Default)]
pub struct Proposals {
    changes: Vec<TeamChange>,
    sent: Vec<u64>,
}

impl Proposals {
    /// The snapshot's changes replace everything.
    pub fn load(&mut self, changes: Vec<TeamChange>) {
        self.changes = changes;
        self.sent.clear();
    }

    /// A change proposed. Whether it is new here.
    pub fn push(&mut self, change: TeamChange) -> bool {
        if self.changes.iter().any(|c| c.id == change.id) {
            return false;
        }
        self.changes.push(change);
        true
    }

    /// No longer waiting, whoever decided.
    pub fn close(&mut self, id: u64) {
        self.changes.retain(|c| c.id != id);
        self.sent.retain(|s| *s != id);
    }

    /// What was sent into a dead link never came back: show it again.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
    }

    /// Every change still waiting, oldest first.
    pub fn open(&self) -> impl Iterator<Item = &TeamChange> {
        self.changes.iter().filter(|c| !self.sent.contains(&c.id))
    }

    pub fn get(&self, id: u64) -> Option<&TeamChange> {
        self.open().find(|c| c.id == id)
    }

    pub fn first(&self) -> Option<&TeamChange> {
        self.open().next()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open().map(|c| c.id).collect()
    }

    /// The person's answer to change `id`: the command to send, the change
    /// hidden at once. `None` when it no longer waits.
    pub fn decide(&mut self, squad: &str, id: u64, approve: bool) -> Option<Command> {
        self.get(id)?;
        self.sent.push(id);
        let squad = squad.to_owned();
        Some(if approve { Command::ApproveTeamChange { squad, id } } else { Command::DeclineTeamChange { squad, id } })
    }
}

// ── the card's words ──

/// A member as the card speaks of it: its name, role, and the ticket it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder<'a> {
    pub name: &'a str,
    pub role: &'a str,
    pub ticket: Option<TicketId>,
}

/// What the change does, in a few words: "mobile 한 명 더 추가", "qa-2 내보내기".
pub fn what(change: &TeamChange) -> String {
    match (change.change, change.count) {
        (Change::Add, 1) => t!("team.add_one", role = change.role),
        (Change::Add, n) => t!("team.add_many", role = change.role, n = n),
        (Change::Remove, _) => t!("team.remove", name = change.member.as_deref().unwrap_or(&change.role)),
    }
}

/// The card's first line: what it does — and, for an add, what waits for it
/// and for how long, and who of the role is busy with what: "mobile 한 명 더
/// 추가 — T-117이 4시간 기다리고 있어요 (mobile은 T-130 중)". `waiting`: how
/// long the first ticket it unblocks has waited, in seconds, when known.
pub fn headline(change: &TeamChange, team: &[Holder<'_>], waiting: Option<u64>) -> String {
    let mut out = what(change);
    if change.change == Change::Remove {
        return out;
    }
    if let Some(&first) = change.unblocks.first() {
        let waits = match waiting.filter(|s| *s >= 60) {
            Some(secs) => t!("team.waits_for", id = first, span = crate::state::short_span(secs)),
            None => t!("team.waits", id = first),
        };
        out = format!("{out} — {waits}");
    }
    let busy: Vec<String> = team.iter().filter(|m| m.role == change.role).filter_map(|m| m.ticket.map(|t| t!("team.busy_on", name = m.name, id = t))).collect();
    if !busy.is_empty() {
        out = format!("{out} ({})", busy.join(", "));
    }
    out
}

/// The model the new members run on, as the card says it: the one asked
/// for, else the role's usual on Claude Code (`opus`, `sonnet`), else the
/// engine's own pick.
pub fn model(change: &TeamChange) -> String {
    let effort = change.effort.as_deref().map(|e| format!(" · {}", t!("team.effort", effort = e))).unwrap_or_default();
    let engine = change.engine.map(|e| format!("{} · ", e.label())).unwrap_or_default();
    let model = match (&change.model, change.engine) {
        (Some(model), _) => model.clone(),
        (None, Some(Engine::Codex)) => t!("team.model_default").to_owned(),
        (None, _) => roles::recommended_model(roles::role_or_custom(&change.role).tier, Engine::Claude, &[]).unwrap_or_default(),
    };
    format!("{engine}{model}{effort}")
}

/// What the new members pick up first: the tickets it unblocks, by number
/// and title.
pub fn first_work(change: &TeamChange, tickets: &[Ticket]) -> Option<String> {
    let list: Vec<String> = change
        .unblocks
        .iter()
        .map(|id| match tickets.iter().find(|t| t.id == *id) {
            Some(t) => format!("T-{id} {}", t.title),
            None => format!("T-{id}"),
        })
        .collect();
    (!list.is_empty()).then(|| list.join(" · "))
}

/// The card's main button: what it does.
pub fn yes_label(change: &TeamChange) -> &'static str {
    match change.change {
        Change::Add => t!("team.approve_add"),
        Change::Remove => t!("team.approve_remove"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add() -> TeamChange {
        TeamChange {
            id: 4,
            change: Change::Add,
            role: "mobile".into(),
            count: 1,
            member: None,
            engine: None,
            model: None,
            effort: None,
            reason: "T-117이 기다려요".into(),
            unblocks: vec![117],
            by: "pm".into(),
            session: 1,
            proposed_at: 0,
            busy_on: None,
        }
    }

    #[test]
    fn a_change_decided_here_hides_at_once_and_is_sent_once() {
        let mut p = Proposals::default();
        p.load(vec![add(), TeamChange { id: 5, ..add() }]);
        assert!(!p.push(add()), "not twice");
        assert_eq!(p.decide("acme", 4, true), Some(Command::ApproveTeamChange { squad: "acme".into(), id: 4 }));
        assert_eq!(p.decide("acme", 4, true), None, "hidden: no second answer");
        assert_eq!(p.ids(), vec![5]);
        assert_eq!(p.decide("acme", 5, false), Some(Command::DeclineTeamChange { squad: "acme".into(), id: 5 }));
        assert!(p.first().is_none());
        // A lost link shows what never came back; the daemon's word takes it for good.
        p.forget_sent();
        assert_eq!(p.ids(), vec![4, 5]);
        p.close(4);
        assert_eq!(p.ids(), vec![5]);
    }

    #[test]
    fn the_headline_says_what_waits_and_who_is_busy() {
        let team = [Holder { name: "pm", role: "pm", ticket: None }, Holder { name: "mobile", role: "mobile", ticket: Some(130) }, Holder { name: "qa", role: "qa", ticket: Some(9) }];
        assert_eq!(headline(&add(), &team, Some(4 * 3600)), "mobile 한 명 더 추가 — T-117이(가) 4시간째 기다리고 있어요 (mobile은(는) T-130 중)");
        assert_eq!(headline(&add(), &team, None), "mobile 한 명 더 추가 — T-117이(가) 기다리고 있어요 (mobile은(는) T-130 중)");
        let bare = TeamChange { unblocks: vec![], count: 2, ..add() };
        assert_eq!(headline(&bare, &team[..1], None), "mobile 2명 더 추가");
        let remove = TeamChange { change: Change::Remove, member: Some("qa".into()), role: "qa".into(), busy_on: Some(9), ..add() };
        assert_eq!(headline(&remove, &team, Some(60)), "qa 내보내기");
        assert_eq!(yes_label(&remove), "내보내기");
        assert_eq!(yes_label(&add()), "추가하기");
    }

    #[test]
    fn the_details_name_the_model_and_the_first_work() {
        assert_eq!(model(&add()), "opus", "mobile is a top-tier role: Claude Code's opus");
        assert_eq!(model(&TeamChange { role: "qa".into(), ..add() }), "sonnet");
        assert_eq!(model(&TeamChange { engine: Some(Engine::Codex), role: "qa".into(), ..add() }), "codex · 엔진 기본값");
        assert_eq!(model(&TeamChange { model: Some("claude-opus-5-5".into()), effort: Some("high".into()), ..add() }), "claude-opus-5-5 · 생각 강도 high");
        let ticket: Ticket = serde_json::from_value(serde_json::json!({ "id": 117, "title": "앱스토어 제출", "body": "", "status": "todo", "depends_on": [], "created_by": "pm" })).unwrap();
        assert_eq!(first_work(&add(), &[ticket]).as_deref(), Some("T-117 앱스토어 제출"));
        assert_eq!(first_work(&add(), &[]).as_deref(), Some("T-117"));
        assert_eq!(first_work(&TeamChange { unblocks: vec![], ..add() }, &[]), None);
    }
}
