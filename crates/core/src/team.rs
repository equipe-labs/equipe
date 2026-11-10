//! Changes to the team the PM proposes (`propose_team_change`) — the tool,
//! and what a call is checked against: plain functions, tested without an
//! engine. A proposal only ever waits for the person; making the change is
//! their command's (`Command::ApproveTeamChange`), never an agent's.

use equipe_protocol::team::{Change, MOST_ADDED, TeamChange};
use equipe_protocol::{Engine, Ticket, TicketId, TicketStatus};
use serde_json::{Value, json};

use crate::board;
use crate::roles;

pub const PROPOSE_TEAM_CHANGE: &str = "propose_team_change";

/// The efforts an engine takes (Claude Code's `--effort`, Codex's reasoning effort).
const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// The PM's tool, as an MCP tool definition.
pub fn tool() -> Value {
    json!({
        "name": PROPOSE_TEAM_CHANGE,
        "description": "Propose a change to the team — add members of a role, or let one go — for the person to approve \
with one click on a card. Call it when work needs a role nobody on the team holds, or when the person says in words to add \
or remove someone (\"add one more mobile\", \"I'll approve adding one\"): never ask them to type a command, and never take \
their words as the approval — the card is where they decide. Queued work is no reason to add: equipe runs a role's ready \
tickets side by side under the members it has. You never add or remove anyone yourself. Approved, the new members join and \
pick up `unblocks` first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "change": { "type": "string", "enum": ["add", "remove"] },
                "role": { "type": "string", "description": "The role to add (designer, frontend, backend, mobile, qa, data, \
or `custom` with `name`), or the role of the member to let go." },
                "count": { "type": "integer", "minimum": 1, "maximum": MOST_ADDED, "description": "How many to add; 1 when left out." },
                "name": { "type": "string", "description": "remove: the member to let go (`mobile-2`), needed when several hold the \
role. add with role custom: the custom role's name (`writer`)." },
                "engine": { "type": "string", "enum": ["claude", "codex"], "description": "What the new members run on; left out, \
equipe picks as it does for /add. Either engine takes any role, changing files included." },
                "model": { "type": "string", "description": "A model the engine offers; left out, the role's usual." },
                "effort": { "type": "string", "enum": EFFORTS, "description": "How hard they think; left out, the engine's default." },
                "reason": { "type": "string", "description": "Why, in one or two lines in the person's language: what waits and for \
how long, who is busy with what." },
                "unblocks": { "type": "array", "items": { "type": "integer" }, "description": "Tickets that wait for these hands; \
the new members take them first." },
                "asked": { "type": "boolean", "description": "true when the person asked in words for this change. Left out, \
an add of a role the team already holds is refused: its members take the role's queued tickets side by side." },
            },
            "required": ["change", "role", "reason"],
        },
    })
}

/// An add the PM makes unasked of a role the team already holds — refused:
/// queued work runs side by side under the role's members as their ticket
/// runs ([`crate::lanes`]), never as more members. Why, for the PM. `None`
/// for a role nobody holds, a removal, or what the person asked for.
pub fn too_soon(p: &Proposal, team: &[Mate<'_>], asked: bool) -> Option<String> {
    if asked || p.change != Change::Add {
        return None;
    }
    let holders: Vec<&str> = team.iter().filter(|m| m.role == p.role).map(|m| m.name).collect();
    if holders.is_empty() {
        return None;
    }
    let next = p.unblocks.first().map(|t| format!("T-{t}")).unwrap_or_else(|| "the queued work".into());
    Some(format!(
        "The team has a {role} already ({names}): equipe starts the role's ready tickets side by side under its members, each in \
a worktree of its own, so {next} needs no new member. Don't add anyone. Propose an add only for a role nobody on the team \
holds, or when the person asks for one (then say `asked`).",
        role = p.role,
        names = holders.join(", "),
    ))
}

/// A member as a proposal is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mate<'a> {
    pub name: &'a str,
    pub role: &'a str,
    /// The ticket it is on.
    pub on: Option<TicketId>,
}

/// A call, checked: everything a [`TeamChange`] holds but who and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub change: Change,
    pub role: String,
    pub count: usize,
    pub member: Option<String>,
    pub engine: Option<Engine>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub reason: String,
    pub unblocks: Vec<TicketId>,
    pub busy_on: Option<TicketId>,
}

impl Proposal {
    pub fn into_change(self, id: u64, by: &str, session: equipe_protocol::SessionId, proposed_at: u64) -> TeamChange {
        TeamChange {
            id,
            change: self.change,
            role: self.role,
            count: self.count,
            member: self.member,
            engine: self.engine,
            model: self.model,
            effort: self.effort,
            reason: self.reason,
            unblocks: self.unblocks,
            by: by.to_owned(),
            session,
            proposed_at,
            busy_on: self.busy_on,
        }
    }
}

fn roles_list() -> String {
    roles::ROLES.iter().filter(|r| r.id != roles::PM).map(|r| r.id).collect::<Vec<_>>().join(", ")
}

/// A `propose_team_change` call checked against the team, the board and
/// what already waits; what is wrong is said so the PM can call again.
pub fn check(args: &Value, team: &[Mate<'_>], tickets: &[Ticket], waiting: &[TeamChange]) -> Result<Proposal, String> {
    let change = match board::arg_str(args, "change") {
        Some("add") => Change::Add,
        Some("remove") => Change::Remove,
        _ => return Err("Say which change: `add` or `remove`.".into()),
    };
    let reason = board::arg_str(args, "reason").ok_or("Give a `reason`: what waits, and why this change helps — in the person's language.")?.to_owned();
    let role_arg = board::arg_str(args, "role").map(|r| r.to_lowercase());
    let name = board::arg_str(args, "name").map(str::to_owned);
    let unblocks = board::arg_ids(args, "unblocks").unwrap_or_default();
    for id in &unblocks {
        match tickets.iter().find(|t| t.id == *id) {
            None => return Err(format!("There is no T-{id}.")),
            Some(t) if t.status.is_closed() => return Err(format!("T-{id} is {} already: it waits for nobody.", t.status.label())),
            Some(_) => {}
        }
    }
    let (role, count, member, busy_on) = match change {
        Change::Add => {
            let role = match (role_arg.as_deref(), name.as_deref()) {
                (Some("custom"), Some(name)) => name.to_lowercase(),
                (Some("custom"), None) => return Err("A custom role needs its name: `name`, like `writer`.".into()),
                (Some(role), _) => role.to_owned(),
                (None, _) => return Err(format!("Which role? {}.", roles_list())),
            };
            let role = if role == "po" { roles::PM.to_owned() } else { role };
            if role == roles::PM {
                return Err("A squad has one PM: you. Propose another role.".into());
            }
            let known = roles::role(&role).is_some() || team.iter().any(|m| m.role == role) || role_arg.as_deref() == Some("custom");
            if !known {
                return Err(format!("No role called {role}. Roles: {} — or `custom` with a `name`.", roles_list()));
            }
            let count = match args.get("count") {
                None | Some(Value::Null) => 1,
                Some(n) => n.as_u64().map(|n| n as usize).filter(|n| (1..=MOST_ADDED).contains(n)).ok_or_else(|| format!("Add 1 to {MOST_ADDED} at once."))?,
            };
            (role, count, None, None)
        }
        Change::Remove => {
            let mate = match (name.as_deref(), role_arg.as_deref()) {
                (Some(name), _) => team.iter().find(|m| m.name == name).ok_or_else(|| format!("Nobody on the squad is called {name}."))?,
                (None, Some(role)) => {
                    let holders: Vec<&Mate<'_>> = team.iter().filter(|m| m.role == role).collect();
                    match holders.as_slice() {
                        [] => return Err(format!("Nobody on the squad is {role}.")),
                        [one] => *one,
                        many => {
                            let names: Vec<&str> = many.iter().map(|m| m.name).collect();
                            return Err(format!("Several members are {role} ({}): give the `name` of the one to let go.", names.join(", ")));
                        }
                    }
                }
                (None, None) => return Err("Who leaves? Give their `name`.".into()),
            };
            if mate.role == roles::PM {
                return Err("The PM stays: a squad always has one.".into());
            }
            (mate.role.to_owned(), 1, Some(mate.name.to_owned()), mate.on)
        }
    };
    let engine = match board::arg_str(args, "engine").map(str::to_lowercase).as_deref() {
        None => None,
        Some("claude") | Some("claude code") => Some(Engine::Claude),
        Some("codex") => Some(Engine::Codex),
        Some(other) => return Err(format!("No engine called {other}: claude or codex.")),
    };
    // Work a member still on the team owns by name waits for that member: a
    // new one would never take it, so naming it is no unblock.
    if change == Change::Add
        && let Some((id, owner)) = unblocks.iter().filter_map(|id| tickets.iter().find(|t| t.id == *id)).find_map(|t| owned_by_name(t, team).map(|o| (t.id, o)))
    {
        return Err(format!(
            "T-{id} is owned by {owner} by name, so a new {role} would not take it: it waits for {owner}. Leave it out of \
`unblocks`, or ask the person to reassign it on its card first."
        ));
    }
    let model = board::arg_str(args, "model").map(str::to_owned);
    let effort = match board::arg_str(args, "effort").map(str::to_lowercase) {
        Some(e) if !EFFORTS.contains(&e.as_str()) => return Err(format!("No effort called {e}: {}.", EFFORTS.join(", "))),
        e => e,
    };
    let same = waiting.iter().find(|w| w.change == change && w.role == role && (change == Change::Add || w.member == member));
    if let Some(same) = same {
        return Err(format!("That change already waits for the person (proposal {}): they decide on its card.", same.id));
    }
    let (engine, model, effort) = if change == Change::Add { (engine, model, effort) } else { (None, None, None) };
    let unblocks = if change == Change::Add { unblocks } else { Vec::new() };
    Ok(Proposal { change, role, count, member, engine, model, effort, reason, unblocks, busy_on })
}

/// Whether a new member of `role` would take `ticket` by the board's rules:
/// owned by the role itself, or by someone no longer on the team whose role
/// it was (their tickets go to their role). A ticket owned by name by a member
/// still on the team (`backend-2`) waits for that member, whatever its role;
/// one owned by another role, for that role. Unowned work is the PM's to give.
pub fn new_member_takes(ticket: &Ticket, role: &str, team: &[Mate<'_>]) -> bool {
    match ticket.owner.as_deref() {
        None => true,
        Some(owner) if owner == role => true,
        Some(owner) if team.iter().any(|m| m.name == owner || m.role == owner) => false,
        Some(owner) => board::role_of(owner) == role,
    }
}

/// The member still on the team that owns `ticket` by name (`backend-2`),
/// if one does. A name that is also a role on the team (`backend`, the
/// role's first member) stands for the role, as on the board.
pub fn owned_by_name<'t>(ticket: &'t Ticket, team: &[Mate<'_>]) -> Option<&'t str> {
    let owner = ticket.owner.as_deref()?;
    (team.iter().any(|m| m.name == owner) && !team.iter().any(|m| m.role == owner)).then_some(owner)
}

/// What becomes of a ticket that still names a member who left the squad
/// ([`left_behind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// In progress in its hands: requeued for whoever takes it next, its
    /// note, branch and worktree kept, as the person's recovery does.
    Requeue,
    /// Anything else unfinished — in review, waiting, to do: it stays as it
    /// is, without the name.
    Rehome,
}

/// The unfinished tickets that still name `name`, a member who left: owned
/// by it, or in its hands.
pub fn left_behind(tickets: &[Ticket], name: &str) -> Vec<(TicketId, Left)> {
    tickets
        .iter()
        .filter(|t| !t.status.is_closed())
        .filter(|t| t.owner.as_deref() == Some(name) || t.assignee.as_deref() == Some(name))
        .map(|t| {
            let held = t.status == TicketStatus::InProgress && t.assignee.as_deref() == Some(name);
            (t.id, if held { Left::Requeue } else { Left::Rehome })
        })
        .collect()
}

/// Who owns a ticket owned by `owner` once member `name`, a `role`, left:
/// the role for what it owned by name; anything else as it was. A member
/// named after its role (`backend`) owned its tickets for the role already.
pub fn owner_after<'a>(owner: Option<&'a str>, name: &str, role: &'a str) -> Option<&'a str> {
    if owner == Some(name) && name != role { Some(role) } else { owner }
}

/// What the PM is told once its proposal waits for the person.
pub fn proposed_reply(p: &Proposal) -> String {
    let what = match p.change {
        Change::Add => {
            let first = match p.unblocks.as_slice() {
                [] => String::new(),
                ids => format!(", and pick up {} first", ids.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ")),
            };
            format!("If they approve, {} {} join{first}.", p.count, p.role)
        }
        Change::Remove => {
            let member = p.member.as_deref().unwrap_or(&p.role);
            match p.busy_on {
                Some(t) => format!("If they approve, {member} stops at its next step and leaves; T-{t} goes back to the board."),
                None => format!("If they approve, {member} leaves."),
            }
        }
    };
    format!(
        "Proposed — the person decides on a card. {what} Don't ask them to type a command, and don't take anything they write \
as the approval: only their click on the card makes the change. You'll hear when they decide; meanwhile carry on."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::{TicketKind, TicketStatus};

    fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
        Ticket {
            id,
            title: format!("ticket {id}"),
            body: String::new(),
            status,
            kind: TicketKind::Task,
            owner: Some("mobile".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None, plan: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![], images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        }
    }

    fn team() -> Vec<Mate<'static>> {
        vec![
            Mate { name: "pm", role: "pm", on: None },
            Mate { name: "mobile", role: "mobile", on: Some(130) },
            Mate { name: "qa", role: "qa", on: None },
            Mate { name: "qa-2", role: "qa", on: None },
            Mate { name: "writer", role: "writer", on: None },
        ]
    }

    fn board() -> Vec<Ticket> {
        vec![ticket(117, TicketStatus::Todo), ticket(130, TicketStatus::InProgress), ticket(90, TicketStatus::Done)]
    }

    #[test]
    fn queued_work_never_adds_a_member_of_a_role_the_team_holds() {
        let add = |unblocks: &[u32]| Proposal { change: Change::Add, role: "backend".into(), count: 1, member: None, engine: None, model: None, effort: None, reason: "r".into(), unblocks: unblocks.to_vec(), busy_on: None };
        // Free, or busy on a ticket for hours: the same answer — its runs take the queue.
        for on in [None, Some(130)] {
            let team = [Mate { name: "backend", role: "backend", on }];
            let why = too_soon(&add(&[117]), &team, false).expect("refused");
            assert!(why.contains("side by side under its members") && why.contains("T-117 needs no new member") && why.contains("Don't add anyone"), "{why}");
        }
        let team = [Mate { name: "backend", role: "backend", on: Some(130) }, Mate { name: "backend-2", role: "backend", on: None }];
        assert!(too_soon(&add(&[]), &team, false).unwrap().contains("(backend, backend-2)"));
        assert_eq!(too_soon(&add(&[117]), &team, true), None, "the person asked");
        assert_eq!(too_soon(&add(&[]), &[], false), None, "a role the team hasn't");
        assert_eq!(too_soon(&add(&[]), &[Mate { name: "qa", role: "qa", on: None }], false), None, "another role isn't this one");
        let remove = Proposal { change: Change::Remove, member: Some("backend".into()), ..add(&[]) };
        assert_eq!(too_soon(&remove, &team, false), None, "letting one go is the person's to weigh");
    }

    fn check_args(args: Value) -> Result<Proposal, String> {
        check(&args, &team(), &board(), &[])
    }

    #[test]
    fn an_add_names_a_role_that_exists_and_what_it_unblocks() {
        let p = check_args(json!({ "change": "add", "role": "Mobile", "reason": "T-117 waits", "unblocks": ["T-117"] })).unwrap();
        assert_eq!((p.change, p.role.as_str(), p.count, p.unblocks.as_slice(), p.member.as_deref()), (Change::Add, "mobile", 1, &[117][..], None));
        let p = check_args(json!({ "change": "add", "role": "frontend", "count": 2, "reason": "r", "engine": "claude", "model": "opus", "effort": "HIGH" })).unwrap();
        assert_eq!((p.count, p.engine, p.model.as_deref(), p.effort.as_deref()), (2, Some(Engine::Claude), Some("opus"), Some("high")));
        // A custom role by its name, as `/add custom writer`; one the team has already.
        assert_eq!(check_args(json!({ "change": "add", "role": "custom", "name": "Translator", "reason": "r" })).unwrap().role, "translator");
        assert_eq!(check_args(json!({ "change": "add", "role": "writer", "reason": "r" })).unwrap().role, "writer");
    }

    #[test]
    fn an_add_that_makes_no_sense_says_why() {
        let err = |args: Value| check_args(args).unwrap_err();
        assert!(err(json!({ "role": "mobile", "reason": "r" })).contains("`add` or `remove`"));
        assert!(err(json!({ "change": "add", "role": "mobile" })).contains("reason"));
        assert!(err(json!({ "change": "add", "role": "astronaut", "reason": "r" })).contains("No role called astronaut. Roles: designer, frontend, backend, mobile, qa, security, marketing, data"));
        assert!(err(json!({ "change": "add", "role": "pm", "reason": "r" })).contains("one PM"));
        assert!(err(json!({ "change": "add", "role": "po", "reason": "r" })).contains("one PM"));
        assert!(err(json!({ "change": "add", "role": "custom", "reason": "r" })).contains("needs its name"));
        assert!(err(json!({ "change": "add", "role": "qa", "count": 9, "reason": "r" })).contains("1 to 3"));
        assert!(err(json!({ "change": "add", "role": "qa", "count": 0, "reason": "r" })).contains("1 to 3"));
        assert!(err(json!({ "change": "add", "role": "mobile", "reason": "r", "unblocks": [5] })).contains("no T-5"));
        assert!(err(json!({ "change": "add", "role": "mobile", "reason": "r", "unblocks": [90] })).contains("T-90 is done already"));
        assert!(err(json!({ "change": "add", "role": "mobile", "reason": "r", "engine": "gpt" })).contains("No engine called gpt"));
        assert!(err(json!({ "change": "add", "role": "mobile", "reason": "r", "effort": "huge" })).contains("No effort called huge"));
        // Either engine takes any role: Codex changes files as Claude does.
        assert!(check_args(json!({ "change": "add", "role": "qa", "reason": "r", "engine": "codex" })).is_ok());
        for role in ["backend", "frontend", "mobile", "designer", "writer"] {
            let p = check_args(json!({ "change": "add", "role": role, "reason": "r", "engine": "codex" })).unwrap();
            assert_eq!(p.engine, Some(Engine::Codex), "{role}");
        }
    }

    #[test]
    fn a_removal_names_one_member_never_the_pm_and_says_when_it_is_at_work() {
        let p = check_args(json!({ "change": "remove", "role": "mobile", "reason": "done for now" })).unwrap();
        assert_eq!((p.member.as_deref(), p.busy_on, p.role.as_str()), (Some("mobile"), Some(130), "mobile"));
        assert!(proposed_reply(&p).contains("mobile stops at its next step and leaves; T-130 goes back to the board"), "{}", proposed_reply(&p));
        let p = check_args(json!({ "change": "remove", "name": "qa-2", "reason": "r", "unblocks": [117], "model": "x" })).unwrap();
        assert_eq!((p.member.as_deref(), p.busy_on, p.unblocks.len(), p.model), (Some("qa-2"), None, 0, None), "a removal unblocks nothing and runs on nothing");
        let err = |args: Value| check_args(args).unwrap_err();
        assert!(err(json!({ "change": "remove", "role": "qa", "reason": "r" })).contains("Several members are qa (qa, qa-2)"));
        assert!(err(json!({ "change": "remove", "name": "pm", "reason": "r" })).contains("The PM stays"));
        assert!(err(json!({ "change": "remove", "role": "pm", "reason": "r" })).contains("The PM stays"));
        assert!(err(json!({ "change": "remove", "name": "ghost", "reason": "r" })).contains("Nobody on the squad is called ghost"));
        assert!(err(json!({ "change": "remove", "role": "data", "reason": "r" })).contains("Nobody on the squad is data"));
        assert!(err(json!({ "change": "remove", "reason": "r" })).contains("Who leaves"));
    }

    /// T-563 review: work owned by name by a member still on the team
    /// (`mobile-2`) was let through as if its role owned it, though a new
    /// `mobile-3` could never take it — no real unblock.
    #[test]
    fn an_add_unblocks_only_work_its_role_owns_never_work_a_member_owns_by_name() {
        let mut team = team();
        team.push(Mate { name: "mobile-2", role: "mobile", on: Some(140) });
        let mut tickets = board();
        let owned = |id: TicketId, owner: &str| {
            let mut t = ticket(id, TicketStatus::InProgress);
            t.owner = Some(owner.into());
            t
        };
        tickets.extend([owned(140, "mobile-2"), owned(141, "qa"), owned(142, "mobile-7"), owned(143, "mobile")]);
        let propose = |unblocks: &[u32]| check(&json!({ "change": "add", "role": "mobile", "reason": "r", "unblocks": unblocks }), &team, &tickets, &[]);
        let by_name = propose(&[140]).unwrap_err();
        assert!(by_name.contains("T-140 is owned by mobile-2 by name") && by_name.contains("waits for mobile-2") && by_name.contains("reassign it on its card"), "{by_name}");
        // Another role's work is no refusal — approved, the board's rules leave it to that role —
        // and `qa`, a member's name that is also its role, is the role.
        assert_eq!(propose(&[141]).unwrap().unblocks, [141]);
        assert_eq!(owned_by_name(&owned(1, "qa"), &team), None);
        assert_eq!(owned_by_name(&owned(1, "mobile-2"), &team), Some("mobile-2"));
        // The role's own work, and a departed member's (it goes to its role), are unblocked.
        assert_eq!(propose(&[142, 143, 117]).unwrap().unblocks, [142, 143, 117]);
        assert!(new_member_takes(&owned(1, "mobile"), "mobile", &team));
        assert!(!new_member_takes(&owned(1, "mobile-2"), "mobile", &team));
        assert!(!new_member_takes(&owned(1, "writer"), "mobile", &team), "a custom role on the team owns its work");
        assert!(new_member_takes(&owned(1, "writer-2"), "writer", &team), "a departed writer's work goes to the role");
    }

    #[test]
    fn the_same_change_is_proposed_once() {
        let waiting = check_args(json!({ "change": "add", "role": "mobile", "reason": "r" })).unwrap().into_change(8, "pm", 1, 0);
        let again = check(&json!({ "change": "add", "role": "mobile", "reason": "again" }), &team(), &board(), std::slice::from_ref(&waiting));
        assert!(again.unwrap_err().contains("already waits for the person (proposal 8)"));
        // Another role, or letting someone go, is another change.
        assert!(check(&json!({ "change": "add", "role": "qa", "reason": "r" }), &team(), &board(), std::slice::from_ref(&waiting)).is_ok());
        assert!(check(&json!({ "change": "remove", "name": "mobile", "reason": "r" }), &team(), &board(), std::slice::from_ref(&waiting)).is_ok());
    }

    #[test]
    fn the_pm_is_told_the_person_decides_on_a_card_and_not_in_words() {
        let p = check_args(json!({ "change": "add", "role": "mobile", "reason": "r", "unblocks": [117] })).unwrap();
        let said = proposed_reply(&p);
        assert!(said.starts_with("Proposed — the person decides on a card."), "{said}");
        assert!(said.contains("1 mobile join, and pick up T-117 first") && said.contains("Don't ask them to type a command"), "{said}");
        assert_eq!(tool()["name"], PROPOSE_TEAM_CHANGE);
        assert_eq!(tool()["inputSchema"]["required"], json!(["change", "role", "reason"]));
    }

    /// What a member that left leaves behind: work in its hands requeued,
    /// anything else it owned rehomed; finished work, and work that names
    /// someone else, untouched.
    #[test]
    fn what_a_member_that_left_leaves_behind() {
        let named = |id: u32, status: TicketStatus, owner: &str, assignee: Option<&str>| Ticket {
            owner: Some(owner.into()),
            assignee: assignee.map(Into::into),
            ..ticket(id, status)
        };
        let board = [
            named(1, TicketStatus::InProgress, "frontend-2", Some("frontend-2")),
            named(2, TicketStatus::InProgress, "frontend", Some("frontend-2")),
            named(3, TicketStatus::InProgress, "frontend-2", Some("frontend")),
            named(4, TicketStatus::Review, "frontend-2", Some("frontend-2")),
            named(5, TicketStatus::Waiting, "frontend", Some("frontend-2")),
            named(6, TicketStatus::Todo, "frontend-2", None),
            named(7, TicketStatus::Idea, "frontend-2", None),
            named(8, TicketStatus::Done, "frontend-2", Some("frontend-2")),
            named(9, TicketStatus::Todo, "frontend-3", None),
        ];
        assert_eq!(
            left_behind(&board, "frontend-2"),
            [(1, Left::Requeue), (2, Left::Requeue), (3, Left::Rehome), (4, Left::Rehome), (5, Left::Rehome), (6, Left::Rehome), (7, Left::Rehome)]
        );
        assert_eq!(owner_after(Some("frontend-2"), "frontend-2", "frontend"), Some("frontend"));
        assert_eq!(owner_after(Some("frontend"), "frontend-2", "frontend"), Some("frontend"), "the role's already");
        assert_eq!(owner_after(Some("designer"), "frontend-2", "frontend"), Some("designer"), "another's stays");
        assert_eq!(owner_after(Some("backend"), "backend", "backend"), Some("backend"), "named after its role: the role's");
    }
}
