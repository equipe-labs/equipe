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

/// Whether a ticket knows where its work happens: a `code` ticket needs its
/// project (or the squad's only one); a `task` or `action` works in the squad
/// home, whatever the squad's projects.
fn knows_where(ticket: &Ticket, single_project: bool) -> bool {
    ticket.kind.in_home() || ticket.project.is_some() || single_project
}

/// Which free member picks up which ready ticket. A ticket is ready when it is
/// `todo`, has an owner, knows where it works, and everything it depends on is
/// finished. The ready ticket highest on the board goes first (the oldest,
/// unless the person reordered it — an urgent one they wrote goes on top);
/// priority decides only between equals; a member takes one ticket at a
/// time; an owner naming a member wins over one naming a role.
pub fn assignments(tickets: &[Ticket], members: &[Candidate<'_>], single_project: bool) -> Vec<(TicketId, String)> {
    let finished = |id: &TicketId| tickets.iter().any(|t| t.id == *id && t.status.is_closed());
    let mut busy: Vec<&str> = members.iter().filter(|m| !m.free).map(|m| m.name).collect();
    let mut out = Vec::new();
    // As the person ordered the board — each owner's queue is its tickets in
    // that order — the more pressing first only between equals.
    let mut ready: Vec<&Ticket> = tickets.iter().filter(|t| t.status == TicketStatus::Todo).collect();
    ready.sort_by_key(|t| (t.order_key(), t.priority, t.id));
    for ticket in ready {
        let Some(owner) = ticket.owner.as_deref() else {
            continue;
        };
        if !knows_where(ticket, single_project) || !ticket.depends_on.iter().all(finished) {
            continue;
        }
        let free = |m: &&Candidate<'_>| !busy.contains(&m.name);
        // A member that has left, or takes nothing new (`frontend-3`), hands its
        // tickets to its role.
        let owner = if members.iter().any(|m| m.role == owner || m.name == owner) { owner } else { role_of(owner) };
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

/// The role a numbered member's name stands for: `frontend-3` → `frontend`.
fn role_of(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((role, n)) if !role.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => role,
        _ => name,
    }
}

/// What a project is called on the board: its directory name.
pub fn project_name(path: &Path) -> String {
    // Screens ask for names every frame; the disk is looked at once per path.
    static NAMES: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, String>>> = std::sync::Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    names.get_or_insert_with(Default::default).entry(path.to_owned()).or_insert_with(|| name_on_disk(path)).clone()
}

fn name_on_disk(path: &Path) -> String {
    let leaf = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    // A folder inside a repository is named with it: `equipe-desktop`.
    if !path.join(".git").exists()
        && let Some(root) = path.ancestors().skip(1).find(|a| a.join(".git").exists())
    {
        return format!("{}-{}", leaf(root), leaf(path));
    }
    leaf(path)
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

/// The first ticket number in `text` that stands on its own — `T-8`, `(T-8)`,
/// `T-8:`, `T-8,`, `T-8와`, `T-8.` closing a sentence, each of `T-2/T-3` —
/// where, and how long. Not one inside a word (`MAT-8`, `T-8a`), a path or a
/// file name (`outputs/T-2 갤러리/`, `T-2.png`, `a_T-2`, `T-2-draft`), or an
/// address (`https://x.y/?t=T-2`): those are not about the ticket.
pub fn ticket_ref(text: &str) -> Option<(usize, usize)> {
    ticket_ref_from(text, 0)
}

/// [`ticket_ref`] from byte `from` on, what comes before it read as context.
pub fn ticket_ref_from(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut from = from;
    while let Some(found) = text.get(from..)?.find("T-") {
        let at = from + found;
        if let Some(len) = number_at(text, at)
            && free_before(text, at)
            && free_after(text, at + len)
            && !in_address(text, at)
        {
            return Some((at, len));
        }
        from = at + 2;
    }
    None
}

/// How long the `T-<digits>` at `at` is, if one is there.
fn number_at(text: &str, at: usize) -> Option<usize> {
    let digits = text.as_bytes().get(at + 2..)?.iter().take_while(|b| b.is_ascii_digit()).count();
    (text[at..].starts_with("T-") && digits > 0).then_some(2 + digits)
}

/// Where the `T-<digits>` that ends at `end` starts, if one ends there.
fn number_ending_at(text: &str, end: usize) -> Option<usize> {
    let digits = text.as_bytes()[..end].iter().rev().take_while(|b| b.is_ascii_digit()).count();
    (digits > 0 && text[..end - digits].ends_with("T-")).then(|| end - digits - 2)
}

/// What paths and file names are joined with: a number touching one is part of the name.
fn joins_name(c: char) -> bool {
    matches!(c, '/' | '\\' | '.' | '_' | '-')
}

/// Nothing of a word or a name runs into the number at `at` from the left; a
/// slash only between two numbers (`T-2/T-3`).
fn free_before(text: &str, at: usize) -> bool {
    match text[..at].chars().next_back() {
        None => true,
        Some('/') => number_ending_at(text, at - 1).is_some_and(|start| free_before(text, start)),
        Some(c) => !(c.is_alphanumeric() || joins_name(c)),
    }
}

/// Nothing of a word or a name runs on from the number ending at `end`: a
/// Korean particle may (`T-2와`), a slash only to another number, a full stop
/// only to end the sentence.
fn free_after(text: &str, end: usize) -> bool {
    match text[end..].chars().next() {
        None => true,
        Some('/') => number_at(text, end + 1).is_some_and(|len| free_after(text, end + 1 + len)),
        Some('.') => text[end..].trim_start_matches('.').chars().next().is_none_or(|c| c.is_whitespace() || !(c.is_alphanumeric() || joins_name(c))),
        Some(c) => !(c.is_ascii_alphanumeric() || joins_name(c)),
    }
}

/// Whether the number at `at` is in a web address written out.
fn in_address(text: &str, at: usize) -> bool {
    let word = text[..at].rsplit(char::is_whitespace).next().unwrap_or_default();
    word.contains("://") || word.starts_with("www.")
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
        if !t.priority.is_normal() {
            facts.push(format!("priority {}", t.priority.label()));
        }
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
        if let Some(qa) = t.design_qa {
            facts.push(format!("design QA on T-{qa} first"));
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

// ── the person's own moves ──

/// What moving a ticket on the board means. The person's command only — a
/// member's word never moves a ticket this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// The same column: only its place changes.
    Reorder,
    /// A proposal taken on: ready to be picked up.
    TakeOn,
    /// Ready work back to a proposal.
    BackToProposal,
    /// Handed-in work accepted.
    Accept,
    Cancel,
    /// What a wait needed is done: back to its member.
    Resume,
    /// Handed-in or accepted work sent back to its member.
    SendBack,
    /// Cancelled work back on the board, as this status.
    Restore(TicketStatus),
}

/// Why a ticket can't be moved there by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Work starts when its owner picks it up: give it an owner instead.
    StartsByItself,
    /// Only the member doing it hands work in.
    HandedInByMember,
    /// Only a member puts work down for the person.
    WaitsByMember,
    /// Only handed-in work is accepted.
    NotHandedIn,
    /// Work under way stops by cancelling it or giving it to someone else.
    UnderWay,
    /// Finished: done work goes back by sending it back to work.
    Closed,
}

impl Refusal {
    /// What its words are called: `board.refuse_<code>` in a screen's words,
    /// `error.refuse_<code>` in the daemon's.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::StartsByItself => "starts_by_itself",
            Refusal::HandedInByMember => "handed_in_by_member",
            Refusal::WaitsByMember => "waits_by_member",
            Refusal::NotHandedIn => "not_handed_in",
            Refusal::UnderWay => "under_way",
            Refusal::Closed => "closed",
        }
    }
}

/// What moving a ticket from `from` to `to` does, or why it can't be done.
pub fn move_rule(from: TicketStatus, to: TicketStatus) -> Result<Move, Refusal> {
    use TicketStatus::*;
    if from == to {
        return Ok(Move::Reorder);
    }
    match (from, to) {
        (Done, Cancelled) => Err(Refusal::Closed),
        (_, Cancelled) => Ok(Move::Cancel),
        (Review, Done) => Ok(Move::Accept),
        (_, Done) => Err(Refusal::NotHandedIn),
        (_, Review) => Err(Refusal::HandedInByMember),
        (_, Waiting) => Err(Refusal::WaitsByMember),
        (Waiting, Todo | InProgress) => Ok(Move::Resume),
        (Review | Done, Todo | InProgress) => Ok(Move::SendBack),
        (Cancelled, Todo | Proposed) => Ok(Move::Restore(to)),
        (Proposed, Todo) => Ok(Move::TakeOn),
        (Todo, Proposed) => Ok(Move::BackToProposal),
        (_, InProgress) => Err(Refusal::StartsByItself),
        (Done, _) => Err(Refusal::Closed),
        _ => Err(Refusal::UnderWay),
    }
}

/// Whether `ticket` waiting on `depends_on` would close a loop of waiting:
/// the loop, from `ticket` back to it, when it would.
pub fn dependency_loop(tickets: &[Ticket], ticket: TicketId, depends_on: &[TicketId]) -> Option<Vec<TicketId>> {
    let deps_of = |id: TicketId| -> Vec<TicketId> {
        if id == ticket { depends_on.to_vec() } else { tickets.iter().find(|t| t.id == id).map(|t| t.depends_on.clone()).unwrap_or_default() }
    };
    // Depth first from what it would wait on, the path kept, until it comes back.
    let mut stack: Vec<Vec<TicketId>> = depends_on.iter().rev().map(|d| vec![ticket, *d]).collect();
    let mut seen: Vec<TicketId> = Vec::new();
    while let Some(path) = stack.pop() {
        let last = path[path.len() - 1];
        if last == ticket {
            return Some(path);
        }
        if seen.contains(&last) {
            continue;
        }
        seen.push(last);
        for next in deps_of(last).into_iter().rev() {
            let mut longer = path.clone();
            longer.push(next);
            stack.push(longer);
        }
    }
    None
}

/// A column's tickets in board order: the person's order, else by number.
pub fn board_order(tickets: &mut [&Ticket]) {
    tickets.sort_by_key(|t| (t.order_key(), t.id));
}

/// The rank that puts a ticket before `before` in `column` (its tickets in
/// board order, without the one moving; `None`: last), or `None` when its
/// neighbours sit too close and the column must be [`spread`] first.
pub fn rank_before(column: &[&Ticket], before: Option<TicketId>) -> Option<i64> {
    use crate::protocol::RANK_STEP;
    // 0 means "never placed" (by its number): a place is never 0.
    let not_zero = |rank: i64, toward: i64| if rank == 0 { toward } else { rank };
    match before.and_then(|b| column.iter().position(|t| t.id == b)) {
        None => Some(column.last().map_or(RANK_STEP, |t| not_zero(t.order_key() + RANK_STEP, 1))),
        Some(0) => Some(not_zero(column[0].order_key() - RANK_STEP, -1)),
        Some(i) => {
            let (lo, hi) = (column[i - 1].order_key(), column[i].order_key());
            let mid = lo + (hi - lo) / 2;
            match mid {
                _ if hi - lo < 2 => None,
                0 if hi > 1 => Some(1),
                0 if lo < -1 => Some(-1),
                0 => None,
                mid => Some(mid),
            }
        }
    }
}

/// A column spread out again, a step apart, in its order: `(ticket, rank)`.
pub fn spread(column: &[&Ticket]) -> Vec<(TicketId, i64)> {
    column.iter().enumerate().map(|(i, t)| (t.id, (i as i64 + 1) * crate::protocol::RANK_STEP)).collect()
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
/// A member puts a decision to the person: questions with options to pick.
pub const ASK_DECISION: &str = "ask_decision";
pub const SHOW_BROWSER: &str = "show_my_browser";
/// Sign in to a site in your browser with a login the person keeps (`vault`).
pub const SIGN_IN: &str = "sign_in";
/// A member writes a fact into the squad's memory.
pub const REMEMBER: &str = "remember";
/// A member takes a wrong or stale fact out of the squad's memory.
pub const FORGET: &str = "forget";
/// A member says when what it is doing will be done — and what it ships, live.
pub const ESTIMATE: &str = "estimate";
/// A designer finishes a design QA ticket: the screens pass, or go back to their engineer.
pub const DESIGN_QA: &str = "design_qa";

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

/// The `images` of an option, and of a hand-in: pictures by their paths.
static IMAGES_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "maxItems": crate::visuals::HAND_IN,
        "description": "Pictures of it — renders, mockups, screenshots you saved: PNG, JPG, WebP, GIF or SVG, by their \
path in your worktree or the squad's folder (relative to where you work, or absolute). At most 4 an option.",
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
                    "proposal": { "type": "boolean", "description": "PM only: put it up as a proposal for the person to take on, rather than ready — when they asked you to shape it." },
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
Your changes are committed to the ticket's branch and the person reviews them. Then end your turn. When the work is \
visual — a logo, a screen, a mockup, an illustration, colours, a layout — name its pictures in `images` (renders or \
screenshots you saved in your worktree), so the person sees them above your summary; images your branch changes \
are shown too. On a code ticket, say whether it changes what users see (`changes_screens`): when the squad has a \
designer, they check those screens from your screenshots before the person reviews.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "What you did and how to check it, in a few lines — in the person's language." },
                    "images": IMAGES_SCHEMA.clone(),
                    "changes_screens": { "type": "boolean", "description": "Code tickets: true when users see a change — a screen, page, layout, style, visible copy, an icon; then give screenshots of each changed screen in `images`. False for changes nobody sees." },
                },
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
                        "description": "Each thing the person has to do or answer, one per item — never several joined into one. \
Each a complete sentence that stands on its own, concrete enough to do without asking; a file or folder named by its path in \
backticks (`outputs/T-12/shots/`). In the person's language.",
                    },
                    "why": { "type": "string", "description": "What it is for, in one line — in the person's language." },
                },
                "required": ["needs"],
            },
        }),
    ];
    if !is_pm {
        tools.push(json!({
            "name": DESIGN_QA,
            "description": "Designers, on a design QA ticket: finish checking an engineer's screens against the spec \
and the design system. `pass` sends their ticket on to the person's review; `changes` sends it back to its engineer \
with your findings. Count only differences from the spec, the tokens and accessibility as defects, not taste. Then \
end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "verdict": { "type": "string", "enum": ["pass", "changes"] },
                    "findings": { "type": "string", "description": "What you checked and found, each difference numbered with where it is, the spec's value, what was built and how much it matters — in the person's language." },
                    "images": IMAGES_SCHEMA.clone(),
                },
                "required": ["verdict", "findings"],
            },
        }));
    }
    tools.push(json!({
        "name": SIGN_IN,
        "description": "Sign in to a site in your browser with a login the person keeps in equipe. You never see or type \
the password: equipe asks the person (unless they already let you use that login), then fills the site's sign-in form \
in your browser and submits it; you hear how it went. Open the site's sign-in page first, or let equipe open it. If \
there's no login kept for the site, ask the person to add one in equipe (Settings › Accounts) — or hand them your \
browser with `show_my_browser` to sign in themselves. Never ask for a password in words, and never read one off a page.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "site": { "type": "string", "description": "The site, by host or address: github.com, https://dash.cloudflare.com/login." },
                "account": { "type": "string", "description": "The username, when the person keeps several for that site." },
                "why": { "type": "string", "description": "What you'll do once signed in, in one line, in the person's language: shown to them on the card." },
            },
            "required": ["site", "why"],
        },
    }));
    tools.push(json!({
        "name": SHOW_BROWSER,
        "description": "Hand your browser to the person — for them to sign in, to look at something with you, or \
to do a step only they can (a code, a CAPTCHA). Always use this rather than only asking in words. Give the page to \
open and, as `why`, exactly what to do there. It opens in their hands in equipe's browser panel (or on their \
screen when this turn ends), with `why` over it; end your turn then. When they hand it back you get a message and \
carry on by yourself. Never run `equipe browser` yourself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The page to open for them." },
                "why": { "type": "string", "description": "Exactly what they should do there, in one line — which site, which account, which button — in the person's language. Shown to them over the page." },
            },
            "required": ["why"],
        },
    }));
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
        "name": ASK_DECISION,
        "description": "Put questions to the person, 1 to 4 at once, each answered in its own place — never write \
them as a numbered list in your reply instead. A decision: a question with 2 to 4 options to pick from, for a real \
choice between distinct options that is theirs to make (an approach, a scope, a provider, a trade-off), not one you \
can settle yourself — the one you recommend first, its label ending in \"(Recommended)\" in the person's language; \
they can also pick several where multi_select is set, or write their own answer. A plain question: leave `options` \
out, and they answer in their own words — for what only they know (a phone number, an email, a name, a date). They \
may skip any. It doesn't wait, and needs no ticket: the person answers when they can, and their answers come to you \
as their next message (\"Answers to your question: 1) … → …\"). Then end your turn, or carry on with work that \
doesn't depend on it. When the ticket you are on can't go on without the answer, or they must do something only \
they can, that is block_on_person; never for a yes before acting in their name (ask_person) or for permissions. A \
visual choice — a logo, colours, a layout, a screen, an icon, an illustration — is never asked in words alone: \
render or mock up each option first (PNG, JPG, WebP, GIF or SVG, in your worktree or the squad's folder), then ask, \
giving each option its `images`; the person sees them side by side and picks from the pictures.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The question, one sentence ending in a question mark — in the person's language." },
                            "header": { "type": "string", "description": "A word or two for it, at most 12 characters: Database, Scope." },
                            "options": {
                                "type": "array",
                                "description": "2 to 4 options to pick from; left out, the person answers in their own words.",
                                "minItems": 2,
                                "maxItems": 4,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "description": "The option in 1 to 5 words." },
                                        "description": { "type": "string", "description": "One line: what it means, or what it costs." },
                                        "images": IMAGES_SCHEMA.clone(),
                                    },
                                    "required": ["label"],
                                },
                            },
                            "multi_select": { "type": "boolean", "description": "Set when several options can be picked together." },
                            "images": {
                                "type": "array",
                                "items": { "type": "string" },
                                "maxItems": crate::visuals::PER_QUESTION,
                                "description": "Pictures of the question as a whole — what is there now, say — shown above its options.",
                            },
                        },
                        "required": ["question"],
                    },
                },
            },
            "required": ["questions"],
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
        "name": ESTIMATE,
        "description": "Tell the person when what you're doing will be done, so they can see it at a glance — call it \
as you start anything that takes more than a couple of minutes, and again whenever your guess changes by much. When \
you push a release, deploy or start anything else that ships, also say when it will be live (CI, a build, a store \
review, DNS…) and what goes live. Honest minutes, not hopeful ones.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "done_in_minutes": { "type": "integer", "description": "Minutes from now until this work is done." },
                "what": { "type": "string", "description": "What is being done, in a few words." },
                "live_in_minutes": { "type": "integer", "description": "Minutes from now until what you are shipping is live; 0 once it is." },
                "live_what": { "type": "string", "description": "What goes live: \"v0.28.1 on Homebrew\", \"equipe.im\"." },
            },
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
        // The team is the person's to change: the PM only proposes.
        tools.push(crate::team::tool());
        // Accepting is the person's: the PM only puts it to them on a card.
        tools.push(crate::accept::tool());
    }
    tools
}

/// What a `waiting` ticket's note says: the person's checklist, and why.
pub fn waiting_note(needs: &[String], why: Option<&str>) -> String {
    crate::protocol::asks::waiting_note(needs, why)
}

/// What a `waiting` ticket's note asks of the person, item by item.
pub fn waiting_needs(note: &str) -> Vec<String> {
    crate::protocol::asks::waiting_needs(note)
}

/// The questions of an `ask_decision` call, checked: 1 to 4, each with a
/// question and 2 to 4 options with distinct labels — or none, for one the
/// person answers in their own words. What is wrong is said so the member
/// can call again.
pub fn questions_arg(args: &Value) -> Result<Vec<crate::protocol::decide::Question>, String> {
    use crate::protocol::decide::{Choice, OPTIONS, QUESTIONS, Question};
    let items = match args.get("questions") {
        Some(Value::Array(items)) => items.clone(),
        // One question given on its own.
        Some(one @ Value::Object(_)) => vec![one.clone()],
        _ => return Err("Give `questions`: 1 to 4, each with a question and 2 to 4 options.".into()),
    };
    if !QUESTIONS.contains(&items.len()) {
        return Err(format!("Ask 1 to 4 questions at once, not {}.", items.len()));
    }
    let mut out = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let n = n + 1;
        let question = arg_str(item, "question").ok_or_else(|| format!("Question {n} has no `question`."))?;
        let options: Vec<Choice> = item
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| match o {
                Value::String(label) => Some(Choice { label: label.trim().to_owned(), ..Choice::default() }),
                o => Some(Choice {
                    label: arg_str(o, "label")?.to_owned(),
                    description: arg_str(o, "description").unwrap_or_default().to_owned(),
                    images: arg_strs(o, "images"),
                }),
            })
            .filter(|c| !c.label.is_empty())
            .collect();
        // None: answered in the person's own words.
        let given = item.get("options").and_then(Value::as_array).is_some_and(|o| !o.is_empty());
        if given && !OPTIONS.contains(&options.len()) {
            return Err(format!("Question {n} needs 2 to 4 options with a label each (or none, for an answer in the person's own words), not {}.", options.len()));
        }
        if options.iter().enumerate().any(|(i, c)| options[..i].iter().any(|d| d.label == c.label)) {
            return Err(format!("Question {n} has two options with the same label."));
        }
        let header: String = arg_str(item, "header").unwrap_or_default().chars().take(24).collect();
        let multi_select = !options.is_empty() && item.get("multi_select").and_then(Value::as_bool).unwrap_or(false);
        // Pictures as the member named them; the runtime finds them where it works.
        let images = arg_strs(item, "images");
        out.push(Question { question: question.to_owned(), header, options, multi_select, images });
    }
    Ok(out)
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

    /// Every ticket number `ticket_ref` finds in `text`, in order.
    fn refs(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some((at, len)) = ticket_ref_from(text, from) {
            out.push(&text[at..at + len]);
            from = at + len;
        }
        out
    }

    #[test]
    fn ticket_numbers_that_stand_on_their_own() {
        let cases: &[(&str, &[&str])] = &[
            ("T-2", &["T-2"]),
            ("see T-12 now", &["T-12"]),
            ("T-2: 갤러리", &["T-2"]),
            ("갤러리 (T-2) 끝", &["T-2"]),
            ("T-2, T-3 and T-4", &["T-2", "T-3", "T-4"]),
            ("T-2와 T-3을 묶습니다", &["T-2", "T-3"]),
            ("T-119처럼", &["T-119"]),
            ("T-2/T-3 둘 다", &["T-2", "T-3"]),
            ("T-2/T-3/T-4", &["T-2", "T-3", "T-4"]),
            ("끝낸 것은 T-2.", &["T-2"]),
            ("T-2. 다음은 T-3...", &["T-2", "T-3"]),
            ("「T-2」 \"T-3\" **T-4**", &["T-2", "T-3", "T-4"]),
            ("T-2!", &["T-2"]),
            ("#T-2", &["T-2"]),
            ("[T-2]", &["T-2"]),
            ("line\nT-2\n", &["T-2"]),
            ("T-8 is ready for you.", &["T-8"]),
            // Inside a path or a file name.
            ("/Users/abel/.equipe/worktrees/photo-studio.home/t2-6ad338ba/outputs/T-2 갤러리 디자인/images/", &[]),
            ("outputs/T-2 갤러리", &[]),
            ("outputs/T-117/shots/", &[]),
            ("/T-2 갤러리", &[]),
            ("a/T-2/T-3", &[]),
            ("T-2/x", &[]),
            ("C:\\work\\T-2", &[]),
            ("T-2\\x", &[]),
            ("T-2.png", &[]),
            ("x.T-2", &[]),
            ("T-2_draft", &[]),
            ("draft_T-2", &[]),
            ("T-2-draft", &[]),
            ("v-T-2", &[]),
            ("T-2./x", &[]),
            // Inside a web address.
            ("https://x.y/T-8", &[]),
            ("https://x.y/?t=T-8 and T-9", &["T-9"]),
            ("www.x.y?t=T-8", &[]),
            // Inside a word.
            ("abcT-2", &[]),
            ("MAT-8", &[]),
            ("UT-3", &[]),
            ("9T-3", &[]),
            ("가T-3", &[]),
            ("T-2a", &[]),
            ("T-2x T-3", &["T-3"]),
            ("T-", &[]),
            ("T-x", &[]),
        ];
        for (text, expected) in cases {
            assert_eq!(&refs(text), expected, "{text:?}");
        }
        assert_eq!(ticket_ref("see T-12 now"), Some((4, 4)));
        assert_eq!(ticket_ref_from("T-2/T-3", 3), Some((4, 3)), "the number before is read as context");
        assert_eq!(ticket_ref_from("a/T-3", 2), None);
    }

    #[test]
    fn a_folder_inside_a_repository_is_named_with_it() {
        let root = std::env::temp_dir().join(format!("equipe-names-{}", std::process::id()));
        let repo = root.join("equipe");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("crates").join("desktop")).unwrap();
        assert_eq!(project_name(&repo), "equipe");
        assert_eq!(project_name(&repo.join("crates").join("desktop")), "equipe-desktop");
        // Not in a repository: its own name.
        assert_eq!(project_name(std::path::Path::new("/nowhere/site")), "site");
        std::fs::remove_dir_all(&root).unwrap();
    }

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
            off_main: None,
            design_qa: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: Vec::new(), images: Vec::new(),
            updated_at: None,
            priority: Default::default(), rank: 0,
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
    fn a_ticket_of_a_member_who_left_goes_to_its_role() {
        let tickets = [ticket(1, "backend-3"), ticket(2, "frontend-2")];
        let members = [free("backend", "backend"), free("frontend-2", "frontend")];
        assert_eq!(assignments(&tickets, &members, true), [(1, "backend".into()), (2, "frontend-2".into())]);
        // Nobody of the role: it waits.
        assert_eq!(assignments(&[ticket(3, "qa-2")], &members, true), []);
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
        assert_eq!(default_kind(None), TicketKind::Task);
        assert_eq!(default_kind(Some(Path::new("/site"))), TicketKind::Code);
    }

    #[test]
    fn the_persons_order_decides_what_goes_first_and_priority_only_between_equals() {
        let normal = ticket(2, "frontend");
        let mut urgent = ticket(3, "frontend");
        urgent.priority = equipe_protocol::Priority::Urgent;
        let mut moved_up = ticket(4, "frontend");
        moved_up.rank = 1; // put above T-2 (2 × 1024) by the person
        let one = [free("frontend", "frontend")];
        assert_eq!(assignments(&[normal.clone(), urgent.clone(), moved_up.clone()], &one, true), [(4, "frontend".into())], "the top of the queue");
        assert_eq!(assignments(&[normal.clone(), urgent.clone()], &one, true), [(2, "frontend".into())], "urgent lower down waits its turn");
        // Placed on the same spot, the more pressing goes first.
        let tie = Ticket { rank: 2048, ..urgent };
        assert_eq!(assignments(&[normal, tie], &one, true), [(3, "frontend".into())]);
    }

    #[test]
    fn a_move_means_what_its_button_would_and_some_are_not_the_persons() {
        use TicketStatus::*;
        assert_eq!(move_rule(Todo, Todo), Ok(Move::Reorder));
        assert_eq!(move_rule(Proposed, Todo), Ok(Move::TakeOn));
        assert_eq!(move_rule(Todo, Proposed), Ok(Move::BackToProposal));
        assert_eq!(move_rule(Review, Done), Ok(Move::Accept));
        assert_eq!(move_rule(InProgress, Cancelled), Ok(Move::Cancel));
        assert_eq!(move_rule(Waiting, Todo), Ok(Move::Resume));
        assert_eq!(move_rule(Review, Todo), Ok(Move::SendBack));
        assert_eq!(move_rule(Done, InProgress), Ok(Move::SendBack));
        assert_eq!(move_rule(Cancelled, Todo), Ok(Move::Restore(Todo)));
        // What only a member does, or only happens by itself.
        assert_eq!(move_rule(Todo, InProgress), Err(Refusal::StartsByItself));
        assert_eq!(move_rule(InProgress, Review), Err(Refusal::HandedInByMember));
        assert_eq!(move_rule(InProgress, Waiting), Err(Refusal::WaitsByMember));
        assert_eq!(move_rule(Todo, Done), Err(Refusal::NotHandedIn));
        assert_eq!(move_rule(InProgress, Todo), Err(Refusal::UnderWay));
        assert_eq!(move_rule(Done, Cancelled), Err(Refusal::Closed));
        assert_eq!(move_rule(Done, Proposed), Err(Refusal::Closed));
    }

    #[test]
    fn a_dependency_that_closes_a_loop_is_found_with_its_path() {
        let mut a = ticket(1, "x");
        a.depends_on = vec![2];
        let mut b = ticket(2, "x");
        b.depends_on = vec![3];
        let c = ticket(3, "x");
        let board = [a, b, c];
        assert_eq!(dependency_loop(&board, 3, &[1]), Some(vec![3, 1, 2, 3]));
        assert_eq!(dependency_loop(&board, 3, &[]), None);
        assert_eq!(dependency_loop(&board, 1, &[3]), None, "waiting on what waits on nothing");
        assert_eq!(dependency_loop(&board, 2, &[2]), Some(vec![2, 2]));
    }

    #[test]
    fn a_place_in_a_column_falls_between_its_neighbours_or_spreads_them_out() {
        let t = |id: TicketId, rank: i64| Ticket { rank, ..ticket(id, "x") };
        let (a, b, c) = (t(1, 0), t(2, 0), t(3, 0)); // 1024, 2048, 3072 by number
        let column = [&a, &b, &c];
        assert_eq!(rank_before(&column, Some(1)), Some(-1), "first: above it — never 0, which means unplaced");
        assert_eq!(rank_before(&[&t(1, 3000)], Some(1)), Some(3000 - 1024));
        assert_eq!(rank_before(&[&t(1, -600), &t(2, 600)], Some(2)), Some(1));
        assert_eq!(rank_before(&column, Some(3)), Some(2560), "between T-2 and T-3");
        assert_eq!(rank_before(&column, None), Some(4096), "last");
        let (x, y) = (t(5, 10), t(6, 11));
        assert_eq!(rank_before(&[&x, &y], Some(6)), None, "no room");
        assert_eq!(spread(&[&x, &y]), vec![(5, 1024), (6, 2048)]);
        let mut column = vec![&c, &a, &x];
        board_order(&mut column);
        assert_eq!(column.iter().map(|t| t.id).collect::<Vec<_>>(), [5, 1, 3]);
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
        // Only the PM proposes changes to the team; nobody has a tool that makes one.
        assert!(names(tools(true)).contains(&crate::team::PROPOSE_TEAM_CHANGE.to_owned()));
        assert!(!names(tools(false)).contains(&crate::team::PROPOSE_TEAM_CHANGE.to_owned()));
        // Only the PM puts acceptance to the person; no member has a tool that accepts or merges.
        assert!(names(tools(true)).contains(&crate::accept::PROPOSE_ACCEPT.to_owned()));
        assert!(!names(tools(false)).contains(&crate::accept::PROPOSE_ACCEPT.to_owned()));
        for name in names(tools(true)) {
            assert!(!["add_member", "remove_member", "approve_team_change"].contains(&name.as_str()), "{name}");
        }
        assert!(names(tools(false)).contains(&SUBMIT_FOR_REVIEW.to_owned()));
        // Design QA is a member's to finish, never the PM's.
        assert!(names(tools(false)).contains(&DESIGN_QA.to_owned()));
        assert!(!names(tools(true)).contains(&DESIGN_QA.to_owned()));
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
    fn a_decision_asks_one_to_four_questions_of_two_to_four_options() {
        let q = |options: Value| json!({ "question": "Which database?", "header": "Database", "options": options });
        let two = json!([{ "label": "Postgres (Recommended)", "description": "Relational" }, { "label": "SQLite" }]);
        let asked = questions_arg(&json!({ "questions": [q(two.clone()), { "question": "Ship what?", "options": ["Auth", "Pay", "Search"], "multi_select": true }] })).unwrap();
        assert_eq!(asked.len(), 2);
        assert_eq!((asked[0].header.as_str(), asked[0].options[0].label.as_str(), asked[0].options[0].description.as_str()), ("Database", "Postgres (Recommended)", "Relational"));
        assert!(asked[1].multi_select && asked[1].options.len() == 3 && asked[1].header.is_empty());
        // One question on its own counts.
        assert_eq!(questions_arg(&json!({ "questions": q(two.clone()) })).unwrap().len(), 1);
        // Without options, the person answers in their own words.
        let free = questions_arg(&json!({ "questions": [{ "question": "Which phone number goes on the EU listing?", "multi_select": true }, { "question": "Email?", "options": [] }] })).unwrap();
        assert!(free.iter().all(|q| q.options.is_empty() && !q.multi_select), "{free:?}");

        let refused = |args: Value| questions_arg(&args).unwrap_err();
        assert!(refused(json!({})).contains("1 to 4"));
        assert!(refused(json!({ "questions": [] })).contains("not 0"));
        assert!(refused(json!({ "questions": vec![q(two.clone()); 5] })).contains("not 5"));
        assert!(refused(json!({ "questions": [q(json!(["only one"]))] })).contains("2 to 4 options"));
        assert!(refused(json!({ "questions": [q(json!(["a", "b", "c", "d", "e"]))] })).contains("not 5"));
        assert!(refused(json!({ "questions": [q(json!(["a", " "]))] })).contains("not 1"), "a blank label isn't an option");
        assert!(refused(json!({ "questions": [q(json!(["a", "a"]))] })).contains("same label"));
        assert!(refused(json!({ "questions": [{ "options": ["a", "b"] }] })).contains("no `question`"));

        // The schema says the same limits, for the PM and members alike.
        for is_pm in [true, false] {
            let tool = tools(is_pm).into_iter().find(|t| t["name"] == ASK_DECISION).unwrap();
            let questions = &tool["inputSchema"]["properties"]["questions"];
            assert_eq!((questions["minItems"].as_u64(), questions["maxItems"].as_u64()), (Some(1), Some(4)));
            let options = &questions["items"]["properties"]["options"];
            assert_eq!((options["minItems"].as_u64(), options["maxItems"].as_u64()), (Some(2), Some(4)));
            let doc = tool["description"].as_str().unwrap();
            assert!(doc.contains("(Recommended)") && doc.contains("block_on_person") && doc.contains("permissions"), "{doc}");
            assert!(doc.contains("leave `options` out") && doc.contains("never write them as a numbered list"), "{doc}");
            assert_eq!(questions["items"]["required"], json!(["question"]));
        }
    }

    #[test]
    fn a_visual_choice_names_its_pictures_on_each_option_and_the_tools_say_to() {
        let asked = questions_arg(&json!({ "questions": [{
            "question": "Which logo?",
            "images": "brand/now.png",
            "options": [
                { "label": "Blue dot (Recommended)", "images": ["brand/a.png", " ", "brand/a-dark.png"] },
                { "label": "Grey dot", "images": "brand/b.svg" },
                "Four blue dots",
            ],
        }] }))
        .unwrap();
        let q = &asked[0];
        assert_eq!(q.images, ["brand/now.png"], "one path given on its own counts");
        assert_eq!(q.options[0].images, ["brand/a.png", "brand/a-dark.png"], "blanks dropped");
        assert_eq!((q.options[1].images.len(), q.options[2].images.len()), (1, 0));
        // The schema has them, and says why; for the PM and members alike.
        for is_pm in [true, false] {
            let tools = tools(is_pm);
            let decide = tools.iter().find(|t| t["name"] == ASK_DECISION).unwrap();
            let item = &decide["inputSchema"]["properties"]["questions"]["items"];
            assert_eq!(item["properties"]["options"]["items"]["properties"]["images"]["type"], "array");
            assert_eq!(item["properties"]["images"]["type"], "array");
            let doc = decide["description"].as_str().unwrap();
            for part in ["logo", "render or mock up each option first", "`images`", "side by side"] {
                assert!(doc.contains(part), "{part}: {doc}");
            }
            let hand_in = tools.iter().find(|t| t["name"] == SUBMIT_FOR_REVIEW).unwrap();
            assert_eq!(hand_in["inputSchema"]["properties"]["images"]["type"], "array");
            let doc = hand_in["description"].as_str().unwrap();
            assert!(doc.contains("`images`") && doc.contains("visual"), "{doc}");
        }
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
