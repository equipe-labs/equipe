//! The board's rules and the tools members use on it — plain functions over
//! tickets, so every rule is tested without an engine.
//!
//! **equipe's rules are code, not an agent.** Who picks up which ticket, in which
//! order, is decided here, the same way every time; a model's word never
//! assigns, merges or approves anything. The PM shapes the board through tools;
//! only the person accepts work.

use std::path::{Path, PathBuf};

use equipe_protocol::{Ticket, TicketId, TicketKind, TicketStatus};
use serde_json::{Value, json};

/// A member as the rules see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub name: &'a str,
    pub role: &'a str,
    /// Neither on a ticket nor in the middle of a turn.
    pub free: bool,
}

/// Ready tickets owned by a role that no member of it was free to take: how
/// many wait, per role — the work extra members of that role would start on.
pub fn unstaffed(tickets: &[Ticket], members: &[Candidate<'_>], single_project: bool) -> Vec<(String, usize)> {
    let taken: Vec<TicketId> = assignments(tickets, members, single_project).into_iter().map(|(id, _)| id).collect();
    let finished = |id: &TicketId| tickets.iter().any(|t| t.id == *id && t.status.is_closed());
    let mut waiting: Vec<(String, usize)> = Vec::new();
    for ticket in tickets.iter().filter(|t| t.status == TicketStatus::Todo && !taken.contains(&t.id)) {
        let Some(owner) = ticket.owner.as_deref().filter(|o| members.iter().any(|m| m.role == *o)) else {
            continue;
        };
        if !knows_where(ticket, single_project) || !ticket.depends_on.iter().all(finished) {
            continue;
        }
        match waiting.iter_mut().find(|(role, _)| role == owner) {
            Some((_, n)) => *n += 1,
            None => waiting.push((owner.to_owned(), 1)),
        }
    }
    waiting
}

/// Whether a ticket knows where its work happens: a `code` ticket needs its
/// project (or the squad's only one); a `task` or `action` works in the squad
/// home, whatever the squad's projects.
fn knows_where(ticket: &Ticket, single_project: bool) -> bool {
    ticket.kind.in_home() || ticket.project.is_some() || single_project
}

/// Which free member picks up which ready ticket. A ticket is ready when it is
/// `todo`, has an owner, knows where it works, and everything it depends on is
/// finished. The oldest ready ticket goes first; a member takes one ticket at a
/// time; an owner naming a member wins over one naming a role.
pub fn assignments(tickets: &[Ticket], members: &[Candidate<'_>], single_project: bool) -> Vec<(TicketId, String)> {
    let finished = |id: &TicketId| tickets.iter().any(|t| t.id == *id && t.status.is_closed());
    let mut busy: Vec<&str> = members.iter().filter(|m| !m.free).map(|m| m.name).collect();
    let mut out = Vec::new();
    for ticket in tickets.iter().filter(|t| t.status == TicketStatus::Todo) {
        let Some(owner) = ticket.owner.as_deref() else {
            continue;
        };
        if !knows_where(ticket, single_project) || !ticket.depends_on.iter().all(finished) {
            continue;
        }
        let free = |m: &&Candidate<'_>| !busy.contains(&m.name);
        let pick = if members.iter().any(|m| m.role == owner) {
            // A role: whoever holds it and is free, the member named after it first.
            members.iter().filter(|m| m.role == owner).filter(free).min_by_key(|m| m.name != owner)
        } else {
            // A member's name (`frontend-2`): that member, when it is free.
            members.iter().filter(|m| m.name == owner).find(free)
        };
        if let Some(member) = pick {
            busy.push(member.name);
            out.push((ticket.id, member.name.to_owned()));
        }
    }
    out
}

/// What a project is called on the board: its directory name.
pub fn project_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The project a tool argument names — by name or by path.
pub fn resolve_project(arg: &str, projects: &[PathBuf]) -> Option<PathBuf> {
    projects.iter().find(|p| project_name(p) == arg || p.as_os_str() == arg).cloned()
}

/// `T-3`, `t3` or `3` → 3.
pub fn parse_ticket_id(text: &str) -> Option<TicketId> {
    let t = text.trim();
    let digits = t.strip_prefix("T-").or_else(|| t.strip_prefix("t-")).or_else(|| t.strip_prefix(['T', 't'])).unwrap_or(t);
    digits.parse().ok()
}

/// The board as text, for a model to read.
pub fn render(tickets: &[Ticket]) -> String {
    let open: Vec<&Ticket> = tickets.iter().filter(|t| !t.status.is_closed()).collect();
    if open.is_empty() {
        return "The board is empty.".into();
    }
    let mut out = String::new();
    for t in open {
        out.push_str(&format!("T-{} [{}] {}", t.id, t.status.label(), t.title));
        let mut facts = vec![t.kind.label().to_owned()];
        if let Some(who) = t.assignee.as_ref().or(t.owner.as_ref()) {
            facts.push(format!("owner {who}"));
        }
        if let Some(project) = &t.project {
            facts.push(format!("project {}", project_name(project)));
        }
        if !t.depends_on.is_empty() {
            let ids: Vec<String> = t.depends_on.iter().map(|id| format!("T-{id}")).collect();
            facts.push(format!("after {}", ids.join(", ")));
        }
        if let Some(branch) = &t.branch {
            facts.push(format!("branch {branch}"));
        }
        if !facts.is_empty() {
            out.push_str(&format!(" — {}", facts.join(", ")));
        }
        out.push('\n');
        if let Some(note) = &t.note {
            out.push_str(&format!("    note: {note}\n"));
        }
    }
    out
}

// ── tools ──

pub const BOARD: &str = "board";
pub const CREATE_TICKET: &str = "create_ticket";
pub const UPDATE_TICKET: &str = "update_ticket";
pub const COMMENT: &str = "comment";
pub const SUBMIT_FOR_REVIEW: &str = "submit_for_review";
/// A member puts its ticket down until the person does something only they can.
pub const BLOCK_ON_PERSON: &str = "block_on_person";
/// A member asks the person's yes before something done in their name.
pub const ASK_PERSON: &str = "ask_person";
/// A member writes a fact into the squad's memory.
pub const REMEMBER: &str = "remember";
/// A member takes a wrong or stale fact out of the squad's memory.
pub const FORGET: &str = "forget";

/// The `kind` argument of `create_ticket` and `update_ticket`.
static KIND_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    json!({
        "type": "string",
        "enum": ["code", "task", "action"],
        "description": "code: changes a repository, on a branch the person merges. task: research, writing, a plan, a \
decision — files in the squad's folder, which go in when the person accepts. action: doing something in an app or the \
browser (replying, signing up, filing) — every outward step asks the person first. Left out: code when the ticket has \
a project, task when it has none.",
    })
});

/// The kind a new ticket gets when none is given: `code` when it has a project, `task` when not.
pub fn default_kind(project: Option<&Path>) -> TicketKind {
    if project.is_some() { TicketKind::Code } else { TicketKind::Task }
}

/// The tools a member gets, as MCP tool definitions. Everyone reads the board,
/// writes tickets (a member's are proposals until the PM or the person accepts
/// them), comments, and hands in its own work; only the PM edits tickets.
pub fn tools(is_pm: bool) -> Vec<Value> {
    let create_doc = if is_pm {
        "Put a ticket on the board. Give it an owner (a role like frontend, or a member's name) and a kind: \
code changes a repository (name the project when the squad has several); task finds out, writes or decides, \
leaving files in the squad's folder; action does something in an app or the browser. equipe assigns it to \
the owner as soon as its dependencies are done and the owner is free."
    } else {
        "Propose a ticket — for work outside your current task, or something you noticed. \
It waits on the board as a proposal until the PM or the person accepts it."
    };
    let mut tools = vec![
        json!({
            "name": BOARD,
            "description": "See the squad's board: every open ticket with its status, owner, project and dependencies.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": CREATE_TICKET,
            "description": create_doc,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "One line: what gets done." },
                    "body": { "type": "string", "description": "What to do and the acceptance criteria." },
                    "owner": { "type": "string", "description": "A role (designer, frontend, backend, qa, data) or a member's name." },
                    "kind": KIND_SCHEMA.clone(),
                    "project": { "type": "string", "description": "The project's name: for code, the repository it changes (needed when the squad has several); for a task or action, what it is about, if anything." },
                    "depends_on": { "type": "array", "items": { "type": "integer" }, "description": "Ticket numbers that must be done first." },
                },
                "required": ["title", "body"],
            },
        }),
        json!({
            "name": COMMENT,
            "description": "Add a note to a ticket's record.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" }, "text": { "type": "string" } },
                "required": ["id", "text"],
            },
        }),
        json!({
            "name": SUBMIT_FOR_REVIEW,
            "description": "Hand in the ticket you are working on, once it meets its acceptance criteria. \
Your changes are committed to the ticket's branch and the person reviews them. Then end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": { "summary": { "type": "string", "description": "What you did and how to check it, in a few lines — in the person's language." } },
                "required": ["summary"],
            },
        }),
        json!({
            "name": BLOCK_ON_PERSON,
            "description": "Put the ticket you are on down when you can't go on without something only the person \
can do — a token or secret to issue, an account or a setting to change. The ticket waits for them with what you \
list as their checklist; you are free for other work meanwhile, and get it back, in the same worktree, once they \
have done it. Your changes so far are kept. Then end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "needs": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Each thing the person has to do, one per item, concrete enough to do without asking — in the person's language.",
                    },
                    "why": { "type": "string", "description": "What it is for, in one line — in the person's language." },
                },
                "required": ["needs"],
            },
        }),
    ];
    tools.push(json!({
        "name": ASK_PERSON,
        "description": "Ask the person's yes before something done in their name that they can't take back \
easily: creating an account or signing up, accepting terms, starting a trial or a paid plan, inviting someone, \
deleting or transferring something, changing security or billing settings. The person sees exactly what you \
wrote and answers yes or no; wait for the answer and do only what they said yes to.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "Exactly what you will do, where — in the person's language." },
                "why": { "type": "string", "description": "Why the work needs it, in one line." },
            },
            "required": ["action", "why"],
        },
    }));
    tools.push(json!({
        "name": REMEMBER,
        "description": "Write a fact into the squad's memory, which every member is given each turn: something about \
the person (a preference, their schedule, a service they use), a decision, a recurring deadline, a term, a contact. \
One fact per call; the same name updates that fact instead of adding another. Never a secret value — an ID, account \
or card number, a password: remember where the original file is instead.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "A short kebab-case name, also its file name: tax-filing." },
                "description": { "type": "string", "description": "The fact in one line — what the index shows — in the person's language." },
                "kind": { "type": "string", "enum": crate::squad_memory::KINDS },
                "body": { "type": "string", "description": "More, when one line isn't enough: details, why, where the original is." },
                "confirmed": { "type": "boolean", "description": "Only when the person agreed to change a fact they wrote last." },
            },
            "required": ["name", "description", "kind"],
        },
    }));
    tools.push(json!({
        "name": FORGET,
        "description": "Take a fact out of the squad's memory, when it turned out wrong or no longer holds.",
        "inputSchema": {
            "type": "object",
            "properties": { "name": { "type": "string", "description": "The fact's name, as the index shows it." } },
            "required": ["name"],
        },
    }));
    // Claude Code's permission prompts come here (`--permission-prompt-tool`):
    // equipe answers from its rules, or asks the person.
    tools.push(json!({
        "name": crate::permissions::APPROVE_TOOL,
        "description": "equipe's answer to a permission prompt: it asks the person when a rule doesn't decide. \
Claude Code calls this itself; there is no need to call it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" },
            },
            "required": ["tool_name", "input"],
        },
    }));
    if is_pm {
        tools.push(json!({
            "name": UPDATE_TICKET,
            "description": "Change a ticket: its title, body, owner, kind, project or dependencies; accept a proposal \
(status todo) or drop one (status cancelled). A ticket waiting on the person goes back to its member with status todo, \
once they have done what it needed. Work in progress and done work are the person's to change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                    "owner": { "type": "string" },
                    "kind": KIND_SCHEMA.clone(),
                    "project": { "type": "string" },
                    "depends_on": { "type": "array", "items": { "type": "integer" } },
                    "status": { "type": "string", "enum": ["todo", "cancelled"] },
                },
                "required": ["id"],
            },
        }));
    }
    tools
}

/// What a `waiting` ticket's note says: the person's checklist, and why.
pub fn waiting_note(needs: &[String], why: Option<&str>) -> String {
    let mut out = String::from("Waiting on you:");
    for need in needs {
        out.push_str(&format!("\n- [ ] {need}"));
    }
    if let Some(why) = why {
        out.push_str(&format!("\n({why})"));
    }
    out
}

/// What a `waiting` ticket's note asks of the person, item by item.
pub fn waiting_needs(note: &str) -> Vec<&str> {
    note.lines().filter_map(|l| l.trim().strip_prefix("- [ ] ")).map(str::trim).collect()
}

/// A list of strings, blanks dropped; a single string counts as one item.
pub fn arg_strs(args: &Value, key: &str) -> Vec<String> {
    let items = match args.get(key) {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(one)) => vec![one.as_str()],
        _ => Vec::new(),
    };
    items.into_iter().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// A string argument, if present and not blank.
pub fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A list of ticket ids, accepting `3` or `"T-3"` for each.
pub fn arg_ids(args: &Value, key: &str) -> Option<Vec<TicketId>> {
    let items = args.get(key)?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as TicketId).or_else(|| v.as_str().and_then(parse_ticket_id)))
            .collect(),
    )
}

pub fn arg_id(args: &Value, key: &str) -> Option<TicketId> {
    let v = args.get(key)?;
    v.as_u64().map(|n| n as TicketId).or_else(|| v.as_str().and_then(parse_ticket_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: TicketId, owner: &str) -> Ticket {
        Ticket {
            id,
            title: format!("ticket {id}"),
            body: String::new(),
            status: TicketStatus::Todo,
            owner: Some(owner.into()),
            assignee: None,
            project: Some("/site".into()),
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            created_by: "pm".into(),
            note: None,
            kind: TicketKind::Code,
        }
    }

    fn free<'a>(name: &'a str, role: &'a str) -> Candidate<'a> {
        Candidate { name, role, free: true }
    }

    #[test]
    fn a_ready_ticket_goes_to_a_free_member_of_its_role() {
        let tickets = [ticket(1, "frontend")];
        assert_eq!(assignments(&tickets, &[free("pm", "pm"), free("frontend", "frontend")], true), [(1, "frontend".into())]);
    }

    #[test]
    fn a_member_takes_one_ticket_at_a_time_oldest_first() {
        let tickets = [ticket(1, "frontend"), ticket(2, "frontend")];
        let members = [free("frontend", "frontend"), free("frontend-2", "frontend")];
        assert_eq!(assignments(&tickets, &members, true), [(1, "frontend".into()), (2, "frontend-2".into())]);
        assert_eq!(assignments(&tickets, &members[..1], true), [(1, "frontend".into())]);
    }

    #[test]
    fn a_busy_member_and_unfinished_dependencies_hold_a_ticket_back() {
        let mut second = ticket(2, "frontend");
        second.depends_on = vec![1];
        let mut first = ticket(1, "designer");
        let tickets = [first.clone(), second.clone()];
        let busy = Candidate { name: "designer", role: "designer", free: false };
        assert_eq!(assignments(&tickets, &[busy, free("frontend", "frontend")], true), []);
        first.status = TicketStatus::Done;
        assert_eq!(assignments(&[first, second], &[free("frontend", "frontend")], true), [(2, "frontend".into())]);
    }

    #[test]
    fn a_ticket_naming_a_member_waits_for_that_member() {
        let tickets = [ticket(1, "frontend-2")];
        let members = [free("frontend", "frontend"), Candidate { name: "frontend-2", role: "frontend", free: false }];
        assert_eq!(assignments(&tickets, &members, true), []);
    }

    #[test]
    fn no_owner_no_project_or_not_todo_means_not_ready() {
        let mut unowned = ticket(1, "x");
        unowned.owner = None;
        let mut nowhere = ticket(2, "frontend");
        nowhere.project = None;
        let mut proposed = ticket(3, "frontend");
        proposed.status = TicketStatus::Proposed;
        let tickets = [unowned, nowhere.clone(), proposed];
        assert_eq!(assignments(&tickets, &[free("frontend", "frontend")], false), []);
        // With one project, a ticket without one means that project.
        assert_eq!(assignments(&[nowhere], &[free("frontend", "frontend")], true), [(2, "frontend".into())]);
    }

    #[test]
    fn work_for_the_squad_home_needs_no_project_however_many_the_squad_has() {
        let mut task = ticket(1, "data");
        task.project = None;
        task.kind = TicketKind::Task;
        let mut action = ticket(2, "pm");
        action.project = None;
        action.kind = TicketKind::Action;
        let mut code = ticket(3, "backend");
        code.project = None;
        let members = [free("data", "data"), free("pm", "pm"), free("backend", "backend")];
        let tickets = [task, action, code];
        assert_eq!(assignments(&tickets, &members, false), [(1, "data".into()), (2, "pm".into())]);
        assert!(unstaffed(&tickets, &members[..1], false).iter().all(|(role, _)| role != "backend"));
        let busy = [Candidate { name: "data", role: "data", free: false }, Candidate { name: "pm", role: "pm", free: false }];
        assert_eq!(unstaffed(&tickets, &busy, false), [("data".into(), 1), ("pm".into(), 1)]);
        assert_eq!(default_kind(None), TicketKind::Task);
        assert_eq!(default_kind(Some(Path::new("/site"))), TicketKind::Code);
    }

    #[test]
    fn ticket_ids_parse_the_ways_people_write_them() {
        for text in ["T-3", "t-3", "T3", "3", " 3 "] {
            assert_eq!(parse_ticket_id(text), Some(3), "{text}");
        }
        assert_eq!(parse_ticket_id("three"), None);
    }

    #[test]
    fn the_board_reads_as_text() {
        let mut t = ticket(2, "frontend");
        t.depends_on = vec![1];
        t.note = Some("needs the logo".into());
        let mut done = ticket(1, "designer");
        done.status = TicketStatus::Done;
        let text = render(&[done, t]);
        assert_eq!(text, "T-2 [todo] ticket 2 — code, owner frontend, project site, after T-1\n    note: needs the logo\n");
        assert_eq!(render(&[]), "The board is empty.");
    }

    #[test]
    fn only_the_pm_edits_tickets() {
        let names = |tools: Vec<Value>| tools.iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert!(names(tools(true)).contains(&UPDATE_TICKET.to_owned()));
        assert!(!names(tools(false)).contains(&UPDATE_TICKET.to_owned()));
        assert!(names(tools(false)).contains(&SUBMIT_FOR_REVIEW.to_owned()));
    }

    #[test]
    fn a_waiting_ticket_lists_what_the_person_has_to_do() {
        let needs = arg_strs(&json!({ "needs": ["GitHub token", " ", "PostHog secrets"] }), "needs");
        assert_eq!(
            waiting_note(&needs, Some("for the deploy")),
            "Waiting on you:\n- [ ] GitHub token\n- [ ] PostHog secrets\n(for the deploy)"
        );
        assert_eq!(waiting_needs(&waiting_note(&needs, None)), ["GitHub token", "PostHog secrets"]);
        assert_eq!(arg_strs(&json!({ "needs": "a token" }), "needs"), ["a token"]);
        assert!(arg_strs(&json!({}), "needs").is_empty());
    }

    #[test]
    fn a_waiting_ticket_is_not_picked_up() {
        let mut t = ticket(1, "backend");
        t.status = TicketStatus::Waiting;
        t.assignee = Some("backend".into());
        let next = ticket(2, "backend");
        assert_eq!(assignments(&[t, next], &[free("backend", "backend")], true), [(2, "backend".into())]);
    }

    #[test]
    fn projects_are_named_by_their_directory() {
        let projects = [PathBuf::from("/code/app"), PathBuf::from("/code/site")];
        assert_eq!(resolve_project("site", &projects), Some(PathBuf::from("/code/site")));
        assert_eq!(resolve_project("/code/app", &projects), Some(PathBuf::from("/code/app")));
        assert_eq!(resolve_project("nope", &projects), None);
    }
}
