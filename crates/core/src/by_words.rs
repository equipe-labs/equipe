//! The person's own words, read for a decision: which waiting request they
//! name ([`read`]), what doing it runs — exactly the commands its card's
//! button sends ([`commands`]) — and equipe's own words for it, read back
//! before it runs ([`read_back`]). The tiers, the yes and the no are the
//! protocol's (`equipe_protocol::by_words`); the runtime keeps what waits
//! (`runtime/words.rs`). Plain functions over what waits, so the rules are
//! tested without a daemon.
//!
//! Nothing a member says reaches any of it: the runtime reads only a
//! screen's [`Command::Say`] — the person's own message — and only its own
//! lines ([`own_words`]).

use std::path::Path;

use equipe_protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};
use equipe_protocol::by_words::{self, Entry, Flag, Kind, Outcome, Reply, Said, Stage, Tier, own_words};
use equipe_protocol::engines;
use equipe_protocol::routines::{Routine, RoutineProposal, Schedule};
use equipe_protocol::team::{Change as TeamMove, TeamChange};
use equipe_protocol::{Approval, Command, Decision, Ticket, TicketEntry, TicketId, TicketKind, TicketStatus};

use crate::accept::{all_of, unfit};
use crate::i18n::{Lang, core};
use crate::permissions;

/// What waits on the person, as the reading sees it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Waiting<'a> {
    pub tickets: &'a [Ticket],
    pub approvals: &'a [Approval],
    pub accepts: &'a [AcceptProposal],
    pub routines: &'a [Routine],
    pub routine_proposals: &'a [RoutineProposal],
    pub team_changes: &'a [TeamChange],
    pub engine_proposals: &'a [engines::Proposal],
}

/// What a ticket is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Accept,
    Merge,
    /// Merge, and push main to `origin`.
    Push,
    KeepOnBranch,
    Idea,
    TakeOn,
}

/// One thing the person's words ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Tickets { verb: Verb, tickets: Vec<TicketId> },
    RunRoutine(u32),
    /// A member's request for permission; `always`, as a rule from now on.
    Approval { id: u64, always: bool },
    /// The PM's card to accept, merge or send back.
    Card(u64),
    ApproveRoutine(u64),
    Team(u64),
    Engine(u64),
}

// ── which kind each request is ──

/// A request for permission's kind, by what allowing it lets through.
pub fn approval_kind(a: &Approval) -> Kind {
    let tool = a.tool.as_str();
    if tool == crate::runtime::CONFIRM {
        return Kind::ActInName;
    }
    if tool == crate::runtime::SIGN_IN_TOOL {
        return Kind::SignInWithLogin;
    }
    if tool == crate::desktop_gate::SEND {
        return Kind::SendAsPerson;
    }
    if tool == crate::controls::SETTING_TOOL {
        return Kind::WidenSetting;
    }
    if permissions::blocked(tool, &a.action) {
        return Kind::Blocked;
    }
    if permissions::SECRETS.iter().any(|s| a.action.contains(s)) {
        return Kind::TouchSecrets;
    }
    if permissions::outward(tool, &a.action) {
        return Kind::AllowOutward;
    }
    Kind::AllowOnce
}

/// Whether "always" can be said to it: a rule can be made of it, and it is
/// no question of the member's own (which is asked every time).
pub fn can_always(a: &Approval) -> bool {
    matches!(approval_kind(a), Kind::AllowOnce | Kind::AllowOutward) && permissions::rule_for(&a.tool, &a.action).is_some()
}

fn ticket<'a>(w: &Waiting<'a>, id: TicketId) -> Option<&'a Ticket> {
    w.tickets.iter().find(|t| t.id == id)
}

fn is_code(t: &Ticket) -> bool {
    !t.kind.in_home() && t.branch.is_some()
}

/// What saying yes to `ask` is, by its kind.
pub fn kind(ask: &Ask, w: &Waiting) -> Kind {
    match ask {
        Ask::Tickets { verb, tickets } => match verb {
            Verb::Accept if tickets.iter().filter_map(|id| ticket(w, *id)).all(|t| t.kind.in_home()) => Kind::AcceptTask,
            Verb::Accept => Kind::AcceptCode,
            Verb::Merge => Kind::Merge,
            Verb::Push => Kind::Push,
            Verb::KeepOnBranch => Kind::KeepOnBranch,
            Verb::Idea => Kind::KeepIdea,
            Verb::TakeOn => Kind::TakeOn,
        },
        Ask::RunRoutine(_) => Kind::RunRoutine,
        Ask::Approval { always: true, .. } => Kind::AllowAlways,
        Ask::Approval { id, .. } => w.approvals.iter().find(|a| a.id == *id).map_or(Kind::Blocked, approval_kind),
        Ask::Card(id) => match w.accepts.iter().find(|c| c.id == *id) {
            Some(card) => match card.verdict {
                Verdict::Changes => Kind::SendBack,
                Verdict::Merge => Kind::Merge,
                Verdict::Accept => kind(&Ask::Tickets { verb: Verb::Accept, tickets: card.tickets.clone() }, w),
            },
            None => Kind::Blocked,
        },
        Ask::ApproveRoutine(_) => Kind::ApproveRoutine,
        Ask::Team(_) => Kind::TeamChange,
        Ask::Engine(_) => Kind::MoveEngine,
    }
}

/// Its tier — a card sending work back is as low as the words that would.
pub fn tier(ask: &Ask, w: &Waiting) -> Tier {
    kind(ask, w).tier()
}

/// Whether it still waits as it did: what it names is all still there to
/// take what it asks.
pub fn still_waits(ask: &Ask, w: &Waiting) -> bool {
    match ask {
        Ask::Tickets { verb, tickets } => !tickets.is_empty() && tickets.iter().all(|id| ticket(w, *id).is_some_and(|t| fits(t, *verb))),
        Ask::RunRoutine(id) => w.routines.iter().any(|r| r.id == *id),
        Ask::Approval { id, .. } => w.approvals.iter().any(|a| a.id == *id),
        Ask::Card(id) => w.accepts.iter().any(|c| c.id == *id),
        Ask::ApproveRoutine(id) => w.routine_proposals.iter().any(|p| p.id == *id),
        Ask::Team(id) => w.team_changes.iter().any(|c| c.id == *id),
        Ask::Engine(id) => w.engine_proposals.iter().any(|p| p.id == *id && !p.applied),
    }
}

/// Whether `t` can be done `verb` to now.
fn fits(t: &Ticket, verb: Verb) -> bool {
    match verb {
        Verb::Accept => unfit(t, Verdict::Accept).is_none() && t.ready_for_review(),
        Verb::Merge | Verb::Push => is_code(t) && unfit(t, Verdict::Merge).is_none() && (t.status != TicketStatus::Review || t.ready_for_review()),
        Verb::KeepOnBranch => t.waits_to_merge(),
        Verb::Idea => t.status == TicketStatus::Proposed,
        Verb::TakeOn => matches!(t.status, TicketStatus::Proposed | TicketStatus::Idea),
    }
}

/// The tickets it is about, for later reversals.
pub fn tickets_of(ask: &Ask, w: &Waiting) -> Vec<TicketId> {
    match ask {
        Ask::Tickets { tickets, .. } => tickets.clone(),
        Ask::Card(id) => w.accepts.iter().find(|c| c.id == *id).map(|c| c.tickets.clone()).unwrap_or_default(),
        Ask::Approval { id, .. } => w.approvals.iter().find(|a| a.id == *id).and_then(|a| a.ticket).into_iter().collect(),
        _ => Vec::new(),
    }
}

// ── reading the words ──

/// `T-1`, `t-42` in the text, in order, once each.
pub fn named_tickets(lower: &str) -> Vec<TicketId> {
    let mut out = Vec::new();
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        let boundary = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        if boundary && chars[i] == 't' && chars[i + 1] == '-' {
            let digits: String = chars[i + 2..].iter().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(id) = digits.parse::<TicketId>()
                && !out.contains(&id)
            {
                out.push(id);
            }
            i += 2 + digits.len();
            continue;
        }
        i += 1;
    }
    out
}

/// Whether any of `key`'s words is written in `text` — as they were matched
/// before words decided anything: anywhere, even inside a word.
fn written(text: &str, key: &str) -> bool {
    equipe_protocol::words::every(key).iter().any(|w| text.contains(w))
}

/// The person's words, read as plainly asking for something that waits:
/// every request they fit — none, one, or several to choose from. A
/// question, a no, an "if", a "later" — anything less plain — is left to
/// the member they said it to, and so are words naming nothing that waits.
pub fn read(text: &str, w: &Waiting) -> Vec<Ask> {
    let words = own_words(text);
    let lower = words.trim().to_lowercase();
    if lower.is_empty() || lower.chars().count() > by_words::MOST_CHARS || lower.starts_with('/') || lower.starts_with('@') {
        return Vec::new();
    }
    if lower.contains(['?', '？']) || written(&lower, "match.unsure") || by_words::says_no(&lower) {
        return Vec::new();
    }
    let has = |key: &str| by_words::any(&lower, key);
    let named = named_tickets(&lower);
    // Tickets: what the words do to them.
    let verb = if written(&lower, "match.merge") || has("match.push") {
        Some(if has("match.push") { Verb::Push } else { Verb::Merge })
    } else if has("match.branch") {
        Some(Verb::KeepOnBranch)
    } else if has("match.idea") {
        Some(Verb::Idea)
    } else if !named.is_empty() && has("match.take_on") {
        Some(Verb::TakeOn)
    } else if written(&lower, "match.accept") {
        Some(Verb::Accept)
    } else {
        None
    };
    if let Some(verb) = verb {
        let ids = if named.is_empty() {
            match verb {
                Verb::Accept | Verb::Merge | Verb::Push if says_all(&lower) => all_of(w.tickets, if verb == Verb::Accept { Verdict::Accept } else { Verdict::Merge }),
                _ => Vec::new(),
            }
        } else {
            named.clone()
        };
        let fit: Vec<TicketId> = ids.into_iter().filter(|id| ticket(w, *id).is_some_and(|t| fits(t, verb))).collect();
        if !fit.is_empty() {
            return vec![Ask::Tickets { verb, tickets: fit }];
        }
    }
    if !named.is_empty() {
        return Vec::new();
    }
    // A routine, by its title: run once, or the PM's turned on.
    let titled = |title: &str| {
        let title = title.trim().to_lowercase();
        title.chars().count() >= 2 && lower.contains(&title)
    };
    if has("match.run") {
        let runs: Vec<Ask> = w.routines.iter().filter(|r| titled(&r.title)).map(|r| Ask::RunRoutine(r.id)).collect();
        if !runs.is_empty() {
            return runs;
        }
    }
    let allow = has("match.allow");
    let always = has("match.always");
    if !(allow || always) {
        return Vec::new();
    }
    let titled_routines: Vec<Ask> = w.routine_proposals.iter().filter(|p| titled(&p.routine.title)).map(|p| Ask::ApproveRoutine(p.id)).collect();
    if !titled_routines.is_empty() {
        return titled_routines;
    }
    // "Allow it", "approve": whatever waits — narrowed to a member, when the words name one.
    let mut all: Vec<(Option<&str>, Ask)> = Vec::new();
    for a in w.approvals {
        if approval_kind(a).reach() != by_words::Reach::Words || (always && !can_always(a)) {
            continue;
        }
        all.push((Some(a.member.as_str()), Ask::Approval { id: a.id, always }));
    }
    if !always {
        all.extend(w.accepts.iter().filter(|c| !c.from_words()).map(|c| (Some(c.by.as_str()), Ask::Card(c.id))));
        all.extend(w.routine_proposals.iter().map(|p| (Some(p.by.as_str()), Ask::ApproveRoutine(p.id))));
        all.extend(w.team_changes.iter().map(|c| (Some(c.by.as_str()), Ask::Team(c.id))));
        all.extend(w.engine_proposals.iter().filter(|p| !p.applied).map(|p| (None, Ask::Engine(p.id))));
    }
    let to: Vec<Ask> = all.iter().filter(|(who, _)| who.is_some_and(|m| holds_name(&lower, m))).map(|(_, a)| a.clone()).collect();
    if to.is_empty() { all.into_iter().map(|(_, a)| a).collect() } else { to }
}

/// "all", "everything" and their kin, in every language (Korean writes "accept all" as one word too).
fn says_all(lower: &str) -> bool {
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '~')).filter(|w| !w.is_empty()).collect();
    let starts = equipe_protocol::words::every("match.all_starts");
    let whole = equipe_protocol::words::every("match.all_words");
    words.iter().any(|w| starts.iter().any(|s| w.starts_with(s)) || *w == "all" || whole.contains(w))
}

fn holds_name(lower: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    name.chars().count() >= 2 && by_words::holds(lower, &name)
}

/// Plainly accepting or merging handed-in work, as a card puts it to the
/// person: the verdict and the tickets — `None` for anything else. What a
/// screen raised a card from before words decided anything, and what the
/// daemon still does where high-risk words take a card.
pub fn card_for(asks: &[Ask]) -> Option<(Verdict, Vec<TicketId>)> {
    match asks {
        [Ask::Tickets { verb: Verb::Accept, tickets }] => Some((Verdict::Accept, tickets.clone())),
        [Ask::Tickets { verb: Verb::Merge | Verb::Push, tickets }] => Some((Verdict::Merge, tickets.clone())),
        _ => None,
    }
}

// ── doing it ──

/// What doing `ask` sends: exactly its card's (or the board's) buttons'
/// commands, as the person's — never more. `said`: how, for the record.
pub fn commands(squad: &str, ask: &Ask, w: &Waiting, said: &Said) -> Vec<Command> {
    let squad = squad.to_owned();
    let said = Some(said.clone());
    match ask {
        Ask::Tickets { verb, tickets } => tickets
            .iter()
            .filter_map(|id| ticket(w, *id))
            .flat_map(|t| {
                let (squad, ticket) = (squad.clone(), t.id);
                match verb {
                    Verb::Accept => vec![Command::Accept { squad, ticket }],
                    Verb::Merge | Verb::Push => {
                        let push = *verb == Verb::Push;
                        let mut out = Vec::new();
                        if t.status == TicketStatus::Review {
                            out.push(Command::Accept { squad: squad.clone(), ticket });
                        }
                        out.push(Command::Merge { squad, ticket, push });
                        out
                    }
                    Verb::KeepOnBranch => vec![Command::KeepOnBranch { squad, ticket }],
                    Verb::Idea => vec![Command::MoveTicket { squad, ticket, status: Some(TicketStatus::Idea), before: None }],
                    Verb::TakeOn => vec![Command::MoveTicket { squad, ticket, status: Some(TicketStatus::Todo), before: None }],
                }
            })
            .collect(),
        Ask::RunRoutine(id) => vec![Command::RunRoutine { squad, id: *id }],
        Ask::Approval { id, always } => vec![Command::Decide { squad, approval: *id, decision: if *always { Decision::Always } else { Decision::Allow }, words: None, said }],
        Ask::Card(id) => {
            let Some(card) = w.accepts.iter().find(|c| c.id == *id) else { return Vec::new() };
            let land = |t: TicketId| (card.verdict == Verdict::Merge && ticket(w, t).is_some_and(is_code)).then_some(Land::Merge);
            vec![Command::ConfirmAccept { squad, id: *id, tickets: card.tickets.iter().map(|t| Confirmed { ticket: *t, merge: land(*t) }).collect(), said }]
        }
        Ask::ApproveRoutine(id) => vec![Command::ApproveRoutine { squad, id: *id }],
        Ask::Team(id) => vec![Command::ApproveTeamChange { squad, id: *id }],
        Ask::Engine(id) => vec![Command::ApplyEngineProposal { squad, id: *id }],
    }
}

// ── saying it ──

/// "T-1, T-3".
pub fn list(ids: &[TicketId]) -> String {
    ids.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ")
}

fn project_of(t: &Ticket) -> String {
    t.project.as_deref().map(crate::board::project_name).unwrap_or_default()
}

fn ticket_line(lang: Lang, t: &Ticket, verb: Option<Verb>) -> String {
    let words = core(lang);
    let base = words.fill("words.item.ticket", &[("id", t.id.to_string()), ("title", t.title.clone())]);
    match (verb, &t.branch) {
        (Some(Verb::Merge | Verb::Push), Some(branch)) => words.fill("words.item.branch", &[("ticket", base), ("branch", branch.clone()), ("project", project_of(t))]),
        _ if t.kind == TicketKind::Task => words.fill("words.item.task", &[("ticket", base)]),
        _ => base,
    }
}

fn schedule(lang: Lang, when: Schedule) -> String {
    let words = core(lang);
    let (hour, minute) = when.hour_minute();
    let time = format!("{hour:02}:{minute:02}");
    match when {
        Schedule::Daily { .. } => words.fill("words.every_day", &[("time", time)]),
        Schedule::Weekly { weekday, .. } => words.fill("words.every_week", &[("day", words.get(&format!("words.weekday.{weekday}")).to_owned()), ("time", time)]),
    }
}

/// One line on what `ask` does — equipe's words, the action and its target
/// (ticket, branch, member, recipient, amount as the request has them) —
/// and, for several things, each in order. Nothing a member summarised:
/// a request's own words are quoted as they came.
pub fn read_back(lang: Lang, ask: &Ask, w: &Waiting) -> (String, Vec<String>) {
    let words = core(lang);
    match ask {
        Ask::Tickets { verb, tickets } => {
            let found: Vec<&Ticket> = tickets.iter().filter_map(|id| ticket(w, *id)).collect();
            let key = match verb {
                Verb::Accept if found.iter().all(|t| t.kind.in_home()) => "words.do.accept",
                Verb::Accept => "words.do.accept_code",
                Verb::Merge => "words.do.merge",
                Verb::Push => "words.do.push",
                Verb::KeepOnBranch => "words.do.keep_on_branch",
                Verb::Idea => "words.do.idea",
                Verb::TakeOn => "words.do.take_on",
            };
            let head = words.fill(key, &[("tickets", list(tickets))]);
            (head, found.iter().map(|t| ticket_line(lang, t, Some(*verb))).collect())
        }
        Ask::RunRoutine(id) => {
            let title = w.routines.iter().find(|r| r.id == *id).map(|r| r.title.clone()).unwrap_or_default();
            (words.fill("words.do.run_routine", &[("title", title)]), Vec::new())
        }
        Ask::ApproveRoutine(id) => match w.routine_proposals.iter().find(|p| p.id == *id) {
            Some(p) => (words.fill("words.do.approve_routine", &[("title", p.routine.title.clone()), ("when", schedule(lang, p.routine.when))]), vec![words.fill("words.item.by", &[("member", p.by.clone()), ("why", p.reason.clone())])]),
            None => (String::new(), Vec::new()),
        },
        Ask::Team(id) => match w.team_changes.iter().find(|c| c.id == *id) {
            Some(c) => {
                let head = match c.change {
                    TeamMove::Add => words.fill("words.do.team_add", &[("count", c.count.to_string()), ("role", c.role.clone())]),
                    TeamMove::Remove => words.fill("words.do.team_remove", &[("member", c.member.clone().unwrap_or_else(|| c.role.clone()))]),
                };
                (head, vec![words.fill("words.item.by", &[("member", c.by.clone()), ("why", c.reason.clone())])])
            }
            None => (String::new(), Vec::new()),
        },
        Ask::Engine(id) => match w.engine_proposals.iter().find(|p| p.id == *id) {
            Some(p) => {
                let members: Vec<String> = p.moves.iter().filter(|m| !m.in_place()).map(|m| m.member.clone()).collect();
                let to = p.to().map(|e| e.label().to_owned()).unwrap_or_default();
                (words.fill("words.do.engine", &[("members", members.join(", ")), ("engine", to)]), Vec::new())
            }
            None => (String::new(), Vec::new()),
        },
        Ask::Card(id) => match w.accepts.iter().find(|c| c.id == *id) {
            Some(card) => {
                let verb = match card.verdict {
                    Verdict::Accept => Verb::Accept,
                    Verdict::Merge => Verb::Merge,
                    Verdict::Changes => {
                        let head = words.fill("words.do.send_back", &[("tickets", list(&card.tickets)), ("note", card.note.clone())]);
                        return (head, vec![words.fill("words.item.proposed", &[("member", card.by.clone())])]);
                    }
                };
                let (head, mut items) = read_back(lang, &Ask::Tickets { verb, tickets: card.tickets.clone() }, w);
                items.push(words.fill("words.item.proposed", &[("member", card.by.clone())]));
                (head, items)
            }
            None => (String::new(), Vec::new()),
        },
        Ask::Approval { id, always } => {
            let Some(a) = w.approvals.iter().find(|a| a.id == *id) else { return (String::new(), Vec::new()) };
            if *always {
                let rule = permissions::rule_for(&a.tool, &a.action).unwrap_or_else(|| a.action.clone());
                return (words.fill("words.do.always", &[("rule", rule)]), vec![words.fill("words.item.asked", &[("member", a.member.clone()), ("action", a.action.clone())])]);
            }
            let key = match approval_kind(a) {
                Kind::ActInName => "words.do.act_in_name",
                Kind::SendAsPerson => "words.do.send",
                Kind::SignInWithLogin => "words.do.sign_in",
                Kind::WidenSetting => "words.do.setting",
                Kind::TouchSecrets => "words.do.secrets",
                Kind::AllowOutward => "words.do.outward",
                _ => "words.do.allow",
            };
            let head = words.fill(key, &[("member", a.member.clone()), ("action", a.action.clone())]);
            let mut items = Vec::new();
            if let Some(t) = a.ticket {
                items.push(words.fill("words.item.on", &[("ticket", format!("T-{t}"))]));
            }
            if let Some(why) = a.reason.as_deref().filter(|r| !r.trim().is_empty()) {
                items.push(words.fill("words.item.why", &[("member", a.member.clone()), ("why", why.to_owned())]));
            }
            (head, items)
        }
    }
}

/// What the person is asked once it is read back.
pub fn ask_line(lang: Lang, tier: Tier) -> String {
    let words = core(lang);
    match tier {
        Tier::Outward => words.fill("words.ask_yes_outward", &[("seconds", (by_words::CANCEL_MS / 1000).to_string())]),
        _ => words.get("words.ask_yes").to_owned(),
    }
}

/// The choice put when the words fit several things: one line each.
pub fn which_items(lang: Lang, asks: &[Ask], w: &Waiting) -> Vec<String> {
    asks.iter().map(|a| read_back(lang, a, w).0).collect()
}

// ── answering what waits ──

/// What the person's next own message does to a request waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Do it now.
    Run,
    /// Confirmed, outward: count down first.
    Countdown,
    /// Read back the one picked (its number, from 1).
    Choose(usize),
    /// Ask again: unclear.
    Again,
    /// It ends so; `pass`: the message goes on to the member too.
    Close { outcome: Outcome, pass: bool },
    /// Not for it: the message goes on, the request as it was.
    Pass,
}

/// How the person's message answers a request at `stage`, of `tier`,
/// choosing among `count`, asked again `asked_again` times — and how it was
/// read, for the log. Deterministic: the word lists, never a model.
pub fn answer(stage: Stage, tier: Tier, text: &str, count: usize, asked_again: u32) -> (Step, Reply) {
    let words = own_words(text);
    let long = words.chars().count() > by_words::MOST_CHARS || words.lines().count() > 2;
    let again = |read: Reply| if asked_again + 1 > by_words::ASK_AGAIN { (Step::Close { outcome: Outcome::Held, pass: false }, read) } else { (Step::Again, read) };
    match stage {
        Stage::Undo | Stage::Countdown => {
            if by_words::stops(text) {
                let outcome = if stage == Stage::Undo { Outcome::Undone } else { Outcome::Cancelled };
                (Step::Close { outcome, pass: false }, if stage == Stage::Undo { Reply::Undo } else { Reply::Cancel })
            } else {
                (Step::Pass, Reply::Other)
            }
        }
        Stage::ReadBack | Stage::Which => {
            if long {
                return (Step::Close { outcome: Outcome::Held, pass: true }, Reply::Other);
            }
            let read = by_words::reply(text);
            if read == Reply::No {
                return (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No);
            }
            if stage == Stage::Which {
                return match by_words::picked(text, count) {
                    Some(n) => (Step::Choose(n), Reply::Pick),
                    None => again(Reply::Unclear),
                };
            }
            match read {
                Reply::Yes if tier == Tier::Outward => (Step::Countdown, Reply::Yes),
                Reply::Yes => (Step::Run, Reply::Yes),
                _ => again(Reply::Unclear),
            }
        }
    }
}

// ── the log ──

/// The words that took back a ticket's acceptance, merge or start later.
const TAKEN_BACK: [&str; 3] = ["reopened", "sent back", "cancelled"];

/// Each entry flagged where it may have gone wrong — its own record, and a
/// ticket it did taken back afterwards (`events`, the board's record).
pub fn flagged(mut entries: Vec<Entry>, events: &[(TicketId, TicketEntry)]) -> Vec<Entry> {
    for entry in &mut entries {
        let mut flags = entry.own_flags();
        let after = entry.closed_at / 1000;
        let reverted = entry.outcome == Outcome::Done && entry.tickets.iter().any(|id| events.iter().any(|(t, e)| t == id && e.at >= after && TAKEN_BACK.contains(&e.kind.as_str())));
        if reverted {
            flags.push(Flag::RevertedLater);
        }
        entry.flags = flags;
    }
    entries
}

/// The log's lines kept in a file, oldest first, one JSON each — any line
/// that doesn't read is skipped.
pub fn parse_log(text: &str) -> Vec<Entry> {
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Where the log is kept: beside the squad's journal, out of every member's reach.
pub fn log_path(home: &Path, squad: &str) -> std::path::PathBuf {
    crate::journal::squad_dir(home, squad).join("words.jsonl")
}

/// The week's line for the retro's facts, when anything was decided in
/// words: how many, and where it went wrong. `None` otherwise — a squad
/// that decides nothing in words has its retro as before.
pub fn week_line(entries: &[Entry], from_ms: u64) -> Option<String> {
    let week: Vec<&Entry> = entries.iter().filter(|e| e.said_at >= from_ms).collect();
    if week.is_empty() {
        return None;
    }
    let count = |f: Flag| week.iter().filter(|e| e.flags.contains(&f)).count();
    let done = week.iter().filter(|e| e.outcome == Outcome::Done).count();
    Some(format!(
        "Decided in the person's words: {} ({} done) · undone {} · cancelled {} · asked again {} · ambiguous {} · held {} · taken back later {}",
        week.len(),
        done,
        count(Flag::Undone),
        count(Flag::Cancelled),
        count(Flag::AskedAgain),
        count(Flag::Ambiguous),
        count(Flag::Held),
        count(Flag::RevertedLater)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::accept::FROM_WORDS;
    use equipe_protocol::by_words::{Heard, Reach, Via};
    use serde_json::json;

    fn ticket(id: TicketId, status: &str, kind: &str) -> Ticket {
        serde_json::from_value(json!({ "id": id, "title": format!("work {id}"), "body": "", "status": status, "kind": kind, "branch": format!("equipe/t-{id}"), "project": "/src/web", "depends_on": [], "created_by": "pm" })).unwrap()
    }

    /// T-1 code and T-2 research in review, T-3 code being done, T-4 code
    /// accepted but not on main, T-5 a proposal, T-6 an idea, T-7 a writing task.
    fn board() -> Vec<Ticket> {
        let mut off = ticket(4, "done", "code");
        off.merged = Some(false);
        vec![ticket(1, "review", "code"), ticket(2, "review", "task"), ticket(3, "in_progress", "code"), off, ticket(5, "proposed", "code"), ticket(6, "idea", "task"), ticket(7, "review", "task")]
    }

    fn approval(id: u64, member: &str, tool: &str, action: &str) -> Approval {
        Approval { id, session: 1, member: member.into(), ticket: Some(3), tool: tool.into(), action: action.into(), reason: Some("to see the tests".into()), preview: None }
    }

    fn routine(id: u32, title: &str) -> Routine {
        Routine { id, title: title.into(), prompt: "go".into(), when: Schedule::Weekly { weekday: 5, hour: 17, minute: 0 }, on: true, last_run: None, next_run: None }
    }

    fn said() -> Said {
        Said { via: Via::Typed, utterance: "yes".into() }
    }

    struct Fixture {
        tickets: Vec<Ticket>,
        approvals: Vec<Approval>,
        accepts: Vec<AcceptProposal>,
        routines: Vec<Routine>,
        proposals: Vec<RoutineProposal>,
        team: Vec<TeamChange>,
    }

    impl Fixture {
        fn new() -> Self {
            Self { tickets: board(), approvals: Vec::new(), accepts: Vec::new(), routines: vec![routine(1, "Weekly report"), routine(2, "주간 리포트")], proposals: Vec::new(), team: Vec::new() }
        }

        fn waiting(&self) -> Waiting<'_> {
            Waiting { tickets: &self.tickets, approvals: &self.approvals, accepts: &self.accepts, routines: &self.routines, routine_proposals: &self.proposals, team_changes: &self.team, engine_proposals: &[] }
        }
    }

    fn tickets(verb: Verb, ids: &[TicketId]) -> Vec<Ask> {
        vec![Ask::Tickets { verb, tickets: ids.to_vec() }]
    }

    #[test]
    fn every_request_for_permission_has_its_kind_and_what_equipe_refuses_stays_refused() {
        let kind = |tool: &str, action: &str| approval_kind(&approval(1, "backend", tool, action));
        assert_eq!(kind("Bash", "npm test"), Kind::AllowOnce);
        assert_eq!(kind("Write", "/src/web/a.rs"), Kind::AllowOnce);
        assert_eq!(kind("WebFetch", "https://example.com"), Kind::AllowOnce);
        assert_eq!(kind(permissions::BROWSER_CODE, "await page.click()"), Kind::AllowOnce);
        assert_eq!(kind("Bash", "git push origin main"), Kind::AllowOutward);
        assert_eq!(kind("Bash", "npm test && vercel deploy --prod"), Kind::AllowOutward);
        assert_eq!(kind("Bash", "cargo publish"), Kind::AllowOutward);
        assert_eq!(kind("Bash", "gh release create v1"), Kind::AllowOutward);
        assert_eq!(kind("Read", "/Users/a/.ssh/id_ed25519"), Kind::TouchSecrets);
        assert_eq!(kind("Bash", "cat .env"), Kind::TouchSecrets);
        assert_eq!(kind(crate::runtime::CONFIRM, "Pay $49.00 a month for Vercel Pro"), Kind::ActInName);
        assert_eq!(kind(crate::runtime::SIGN_IN_TOOL, "github.com as abel"), Kind::SignInWithLogin);
        assert_eq!(kind(crate::desktop_gate::SEND, "Messages → Mina: “see you at 5”"), Kind::SendAsPerson);
        assert_eq!(kind(crate::controls::SETTING_TOOL, "Mode: full"), Kind::WidenSetting);
        assert_eq!(kind("Bash", "sudo rm -rf /"), Kind::Blocked);
        assert_eq!(kind("Bash", "git push --force origin main"), Kind::Blocked);
        assert_eq!(Kind::Blocked.reach(), Reach::Never);
        // Only plain commands and sites can become a rule; a member's own question never does.
        assert!(can_always(&approval(1, "b", "Bash", "npm test")));
        assert!(!can_always(&approval(1, "b", "Bash", "npm test && rm x")));
        assert!(!can_always(&approval(1, "b", crate::runtime::CONFIRM, "Sign up")));
        assert!(!can_always(&approval(1, "b", crate::desktop_gate::SEND, "Mail")));
    }

    #[test]
    fn every_ask_has_its_tier() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(10, "backend", "Bash", "npm test"), approval(11, "backend", "Bash", "git push"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49")];
        f.accepts = vec![AcceptProposal { id: 20, verdict: Verdict::Merge, tickets: vec![1], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 }];
        let w = f.waiting();
        let t = |ask: Ask| tier(&ask, &w);
        assert_eq!(t(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 7] }), Tier::Low, "research and writing");
        assert_eq!(t(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 1] }), Tier::High, "code among them");
        assert_eq!(t(Ask::Tickets { verb: Verb::Merge, tickets: vec![1] }), Tier::High);
        assert_eq!(t(Ask::Tickets { verb: Verb::Push, tickets: vec![1] }), Tier::Outward);
        assert_eq!(t(Ask::Tickets { verb: Verb::Idea, tickets: vec![5] }), Tier::Low);
        assert_eq!(t(Ask::Tickets { verb: Verb::TakeOn, tickets: vec![5] }), Tier::Low);
        assert_eq!(t(Ask::Tickets { verb: Verb::KeepOnBranch, tickets: vec![4] }), Tier::Low);
        assert_eq!(t(Ask::RunRoutine(1)), Tier::Low);
        assert_eq!(t(Ask::ApproveRoutine(1)), Tier::High);
        assert_eq!(t(Ask::Engine(1)), Tier::High);
        assert_eq!(t(Ask::Team(1)), Tier::Outward);
        assert_eq!(t(Ask::Approval { id: 10, always: false }), Tier::High);
        assert_eq!(t(Ask::Approval { id: 10, always: true }), Tier::Outward, "widening permissions");
        assert_eq!(t(Ask::Approval { id: 11, always: false }), Tier::Outward);
        assert_eq!(t(Ask::Approval { id: 12, always: false }), Tier::Outward, "paying");
        assert_eq!(t(Ask::Card(20)), Tier::High);
        assert_eq!(t(Ask::Approval { id: 99, always: false }), Tier::Outward, "gone: as refused");
    }

    #[test]
    fn plain_words_name_what_waits_and_anything_less_plain_is_left_to_the_member() {
        let f = Fixture::new();
        let w = f.waiting();
        let read = |text: &str| read(text, &w);
        // Accepting, the way a card was raised before.
        assert_eq!(read("T-2 수락"), tickets(Verb::Accept, &[2]));
        assert_eq!(read("accept T-2 and T-7"), tickets(Verb::Accept, &[2, 7]));
        assert_eq!(read("전부다 수락할게."), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read("다수락해줘"), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read("Accept them all!"), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read("T-1 머지"), tickets(Verb::Merge, &[1]));
        assert_eq!(read("merge T-4"), tickets(Verb::Merge, &[4]));
        assert_eq!(read("merge T-1 and push"), tickets(Verb::Push, &[1]));
        assert_eq!(read("T-1 푸시까지"), tickets(Verb::Push, &[1]));
        assert_eq!(read("전부 머지까지 해줘"), tickets(Verb::Merge, &[1, 4]), "to merge, accepted code off main too; research has nothing to merge");
        assert_eq!(read("leave T-4 on its branch"), tickets(Verb::KeepOnBranch, &[4]));
        assert_eq!(read("T-5 아이디어로 남겨"), tickets(Verb::Idea, &[5]));
        assert_eq!(read("keep T-5 as an idea"), tickets(Verb::Idea, &[5]));
        assert_eq!(read("take on T-6"), tickets(Verb::TakeOn, &[6]));
        assert_eq!(read("T-5 진행"), tickets(Verb::TakeOn, &[5]));
        assert_eq!(read("run the weekly report"), vec![Ask::RunRoutine(1)]);
        assert_eq!(read("주간 리포트 한 번 돌려 줘"), vec![Ask::RunRoutine(2)]);
        // Named but not fit, or nothing named: nothing.
        assert_eq!(read("T-3 수락"), vec![], "being done");
        assert_eq!(read("T-1, T-3, T-99 수락"), tickets(Verb::Accept, &[1]));
        assert_eq!(read("수락할게"), vec![]);
        assert_eq!(read("run it"), vec![], "no routine named");
        for unsure in [
            "전부 수락할까?",
            "T-1 수락하지 마",
            "아직 수락하지 말아줘",
            "테스트 통과하면 다 수락할게",
            "수락 버튼 어디 있어",
            "T-1 수락 버튼이 안 눌려요",
            "이건 반려",
            "T-1 어때",
            "don't accept T-1",
            "accept all after the tests",
            "don't merge T-1",
            "not yet, merge T-1 later",
            "/accept T-1",
            "@pm accept T-1",
            "다 했어요",
            "오늘 한 일 정리해서 내일 아침에 다시 보여줘. 그리고 T-1은 리뷰가 끝나면 수락할 생각이에요, 문제 없으면요",
            "まだ承認しないで",
            "不要接受 T-1",
            "先别合并",
            "no aceptes T-1",
            "n'accepte pas T-1",
            "T-1 nicht akzeptieren",
            "wenn die Tests laufen, alles akzeptieren",
        ] {
            assert_eq!(read(unsure), vec![], "{unsure}");
        }
        // Quoted or fenced words are never the person's.
        assert_eq!(read("> merge T-1\nok, thanks"), vec![]);
        assert_eq!(read("<!-- reply 1 -->\n> accept T-2\n<!-- reply -->\nlooks fine"), vec![]);
        assert_eq!(read("```\nmerge T-1\n```"), vec![]);
    }

    #[test]
    fn plain_words_are_read_in_every_language() {
        let f = Fixture::new();
        let w = f.waiting();
        for (text, verb, ids) in [
            ("T-2 accept", Verb::Accept, vec![2]),
            ("T-2 承認して", Verb::Accept, vec![2]),
            ("全部承認", Verb::Accept, vec![1, 2, 7]),
            ("T-1 マージして", Verb::Merge, vec![1]),
            ("接受 T-2", Verb::Accept, vec![2]),
            ("全部接受", Verb::Accept, vec![1, 2, 7]),
            ("T-1 合并", Verb::Merge, vec![1]),
            ("T-1 合併", Verb::Merge, vec![1]),
            ("aprueba T-2", Verb::Accept, vec![2]),
            ("acepta todo", Verb::Accept, vec![1, 2, 7]),
            ("fusiona T-1", Verb::Merge, vec![1]),
            ("T-2 validé", Verb::Accept, vec![2]),
            ("accepte tout", Verb::Accept, vec![1, 2, 7]),
            ("fusionne T-1", Verb::Merge, vec![1]),
            ("T-2 genehmigen", Verb::Accept, vec![2]),
            ("alles akzeptieren", Verb::Accept, vec![1, 2, 7]),
            ("T-1 zusammenführen", Verb::Merge, vec![1]),
        ] {
            assert_eq!(read(text, &w), tickets(verb, &ids), "{text}");
        }
    }

    #[test]
    fn allowing_in_words_names_what_waits_and_several_are_a_choice() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(10, "backend", "Bash", "npm test"), approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49")];
        f.accepts = vec![
            AcceptProposal { id: 20, verdict: Verdict::Accept, tickets: vec![1], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 },
            AcceptProposal { id: 21, verdict: Verdict::Accept, tickets: vec![2], note: String::new(), summary: String::new(), by: FROM_WORDS.into(), session: 1, proposed_at: 0 },
        ];
        f.proposals = vec![RoutineProposal { id: 30, routine: routine(0, "Dependency check"), reason: "weekly".into(), by: "pm".into(), session: 1, proposed_at: 0 }];
        let w = f.waiting();
        assert_eq!(read("allow backend", &w), vec![Ask::Approval { id: 10, always: false }]);
        assert_eq!(read("frontend 허용", &w), vec![Ask::Approval { id: 11, always: false }]);
        assert_eq!(read("approve the dependency check", &w), vec![Ask::ApproveRoutine(30)]);
        // Everything waiting, with no name: the person chooses. A card raised from their own words is theirs to click.
        assert_eq!(
            read("승인", &w),
            vec![Ask::Approval { id: 10, always: false }, Ask::Approval { id: 11, always: false }, Ask::Approval { id: 12, always: false }, Ask::Card(20), Ask::ApproveRoutine(30)]
        );
        // "Always": only what can become a rule.
        assert_eq!(read("always allow backend", &w), vec![Ask::Approval { id: 10, always: true }]);
        assert_eq!(read("앞으로 항상 허용", &w), vec![Ask::Approval { id: 10, always: true }, Ask::Approval { id: 11, always: true }]);
        assert_eq!(read("don't allow backend", &w), vec![]);
        assert_eq!(read("allow backend?", &w), vec![]);
        // Nothing waits: nothing to read.
        let empty = Fixture { approvals: vec![], accepts: vec![], proposals: vec![], ..Fixture::new() };
        assert_eq!(read("allow it", &empty.waiting()), vec![]);
    }

    #[test]
    fn doing_it_sends_only_what_its_card_or_the_boards_buttons_send() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(10, "backend", "Bash", "npm test")];
        f.accepts = vec![AcceptProposal { id: 20, verdict: Verdict::Merge, tickets: vec![1, 2], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 }];
        let w = f.waiting();
        let run = |ask: Ask| commands("acme", &ask, &w, &said());
        let s = || "acme".to_owned();
        assert_eq!(run(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 7] }), [Command::Accept { squad: s(), ticket: 2 }, Command::Accept { squad: s(), ticket: 7 }]);
        assert_eq!(run(Ask::Tickets { verb: Verb::Merge, tickets: vec![1, 4] }), [Command::Accept { squad: s(), ticket: 1 }, Command::Merge { squad: s(), ticket: 1, push: false }, Command::Merge { squad: s(), ticket: 4, push: false }]);
        assert_eq!(run(Ask::Tickets { verb: Verb::Push, tickets: vec![4] }), [Command::Merge { squad: s(), ticket: 4, push: true }]);
        assert_eq!(run(Ask::Tickets { verb: Verb::KeepOnBranch, tickets: vec![4] }), [Command::KeepOnBranch { squad: s(), ticket: 4 }]);
        assert_eq!(run(Ask::Tickets { verb: Verb::Idea, tickets: vec![5] }), [Command::MoveTicket { squad: s(), ticket: 5, status: Some(TicketStatus::Idea), before: None }]);
        assert_eq!(run(Ask::Tickets { verb: Verb::TakeOn, tickets: vec![6] }), [Command::MoveTicket { squad: s(), ticket: 6, status: Some(TicketStatus::Todo), before: None }]);
        assert_eq!(run(Ask::RunRoutine(2)), [Command::RunRoutine { squad: s(), id: 2 }]);
        assert_eq!(run(Ask::Approval { id: 10, always: false }), [Command::Decide { squad: s(), approval: 10, decision: Decision::Allow, words: None, said: Some(said()) }]);
        assert_eq!(run(Ask::Approval { id: 10, always: true }), [Command::Decide { squad: s(), approval: 10, decision: Decision::Always, words: None, said: Some(said()) }]);
        assert_eq!(
            run(Ask::Card(20)),
            [Command::ConfirmAccept { squad: s(), id: 20, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Merge) }, Confirmed::only(2)], said: Some(said()) }],
            "merged where there is code; research goes into the folder"
        );
        assert_eq!(run(Ask::ApproveRoutine(30)), [Command::ApproveRoutine { squad: s(), id: 30 }]);
        assert_eq!(run(Ask::Team(40)), [Command::ApproveTeamChange { squad: s(), id: 40 }]);
        assert_eq!(run(Ask::Engine(50)), [Command::ApplyEngineProposal { squad: s(), id: 50 }]);
        // Never any other command: nothing that pairs, saves a secret, widens a mode or turns words on.
        for ask in [Ask::Tickets { verb: Verb::Push, tickets: vec![1, 2, 4] }, Ask::Card(20), Ask::Approval { id: 10, always: true }] {
            for c in run(ask) {
                assert!(matches!(c, Command::Accept { .. } | Command::Merge { .. } | Command::ConfirmAccept { .. } | Command::Decide { .. }), "{c:?}");
            }
        }
    }

    #[test]
    fn a_request_still_waits_only_while_what_it_names_takes_it() {
        let mut f = Fixture::new();
        let ask = Ask::Tickets { verb: Verb::Accept, tickets: vec![2] };
        assert!(still_waits(&ask, &f.waiting()));
        f.tickets[1].status = TicketStatus::Done;
        assert!(!still_waits(&ask, &f.waiting()), "accepted on the board meanwhile");
        assert!(!still_waits(&Ask::Approval { id: 10, always: false }, &f.waiting()));
        assert!(still_waits(&Ask::RunRoutine(1), &f.waiting()));
    }

    #[test]
    fn the_read_back_says_the_action_and_its_target_in_equipes_own_words() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49.00 a month to Vercel")];
        let w = f.waiting();
        let (head, items) = read_back(Lang::En, &Ask::Tickets { verb: Verb::Push, tickets: vec![1] }, &w);
        assert_eq!(head, "Merge T-1 into main and push main to origin");
        assert_eq!(items, ["T-1 “work 1” — equipe/t-1 → main of web"]);
        let (head, items) = read_back(Lang::Ko, &Ask::Tickets { verb: Verb::Merge, tickets: vec![1] }, &w);
        assert_eq!(head, "T-1를 이 Mac에서 main에 머지");
        assert_eq!(items, ["T-1 “work 1” — equipe/t-1 → web의 main"]);
        let (head, items) = read_back(Lang::En, &Ask::Tickets { verb: Verb::Accept, tickets: vec![2] }, &w);
        assert_eq!((head.as_str(), items[0].as_str()), ("Accept T-2", "T-2 “work 2” — into the squad's folder"));
        let (head, items) = read_back(Lang::En, &Ask::Approval { id: 11, always: false }, &w);
        assert_eq!(head, "Let frontend reach outside the squad, once: git push origin main");
        assert_eq!(items, ["On T-3", "frontend says: “to see the tests”"]);
        // The amount as the member asked it, word for word.
        let (head, _) = read_back(Lang::En, &Ask::Approval { id: 12, always: false }, &w);
        assert_eq!(head, "Let pm act in your name: “Pay $49.00 a month to Vercel”");
        let (head, _) = read_back(Lang::Ko, &Ask::Approval { id: 12, always: false }, &w);
        assert_eq!(head, "pm가 내 이름으로 해요: “Pay $49.00 a month to Vercel”");
        assert_eq!(read_back(Lang::En, &Ask::RunRoutine(1), &w).0, "Run the routine “Weekly report” once, now");
        assert_eq!(ask_line(Lang::En, Tier::Outward), "Say yes and it goes in 10 s — say cancel to stop it then.");
        assert_eq!(ask_line(Lang::En, Tier::High), "Say yes to go ahead, or no to leave it on its card.");
    }

    #[test]
    fn a_read_back_runs_only_on_a_clear_yes_and_asks_again_when_unclear() {
        let rb = |text: &str, tier: Tier| answer(Stage::ReadBack, tier, text, 0, 0);
        assert_eq!(rb("yes", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb("ㄱㄱ", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb("고고", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb("ok", Tier::Outward), (Step::Countdown, Reply::Yes), "outward: counts down first");
        assert_eq!(rb("no", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(rb("아니 하지 마", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(rb("not yet", Tier::Outward), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(rb("hmm", Tier::High), (Step::Again, Reply::Unclear));
        assert_eq!(rb("yes?", Tier::High), (Step::Again, Reply::Unclear));
        assert_eq!(rb("> yes", Tier::High), (Step::Again, Reply::Unclear), "a quoted yes is not theirs");
        // Unclear too often: held, nothing done.
        assert_eq!(answer(Stage::ReadBack, Tier::High, "hmm", 0, by_words::ASK_AGAIN), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::Unclear));
        // Moved on to something else entirely: held, and the words go on to the member.
        let long = "actually, first fix the header on the pricing page and then look at the footer links again";
        assert_eq!(rb(long, Tier::High), (Step::Close { outcome: Outcome::Held, pass: true }, Reply::Other));
        // Choosing: a number, else asked again; a no holds.
        assert_eq!(answer(Stage::Which, Tier::High, "2", 3, 0), (Step::Choose(2), Reply::Pick));
        assert_eq!(answer(Stage::Which, Tier::High, "yes", 3, 0), (Step::Again, Reply::Unclear), "a yes chooses nothing");
        assert_eq!(answer(Stage::Which, Tier::High, "none of them, no", 3, 0), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
    }

    #[test]
    fn undo_and_cancel_windows_stop_on_any_no_and_let_the_rest_through() {
        assert_eq!(answer(Stage::Undo, Tier::Low, "undo", 0, 0), (Step::Close { outcome: Outcome::Undone, pass: false }, Reply::Undo));
        assert_eq!(answer(Stage::Undo, Tier::Low, "되돌려", 0, 0).0, Step::Close { outcome: Outcome::Undone, pass: false });
        assert_eq!(answer(Stage::Undo, Tier::Low, "and fix the header", 0, 0), (Step::Pass, Reply::Other));
        assert_eq!(answer(Stage::Countdown, Tier::Outward, "cancel", 0, 0), (Step::Close { outcome: Outcome::Cancelled, pass: false }, Reply::Cancel));
        assert_eq!(answer(Stage::Countdown, Tier::Outward, "잠깐", 0, 0).0, Step::Close { outcome: Outcome::Cancelled, pass: false });
        assert_eq!(answer(Stage::Countdown, Tier::Outward, "ok", 0, 0), (Step::Pass, Reply::Other));
    }

    fn entry(id: u64, outcome: Outcome, tickets: Vec<TicketId>, closed_at: u64) -> Entry {
        Entry {
            id,
            session: 1,
            kind: Kind::AcceptTask,
            tier: Tier::Low,
            via: Via::Typed,
            all_by_words: false,
            said: "T-2 수락".into(),
            read_back: String::new(),
            items: vec![],
            answers: vec![],
            tickets,
            said_at: closed_at - 5_000,
            read_back_at: None,
            decided_at: None,
            closed_at,
            outcome,
            asked_again: 0,
            chose: false,
            device: None,
            flags: vec![],
        }
    }

    #[test]
    fn the_log_flags_what_was_undone_cancelled_unclear_or_taken_back_later() {
        let back = |ticket: TicketId, at: u64, kind: &str| (ticket, TicketEntry { at, by: "you".into(), kind: kind.into(), text: String::new() });
        let mut unclear = entry(4, Outcome::Done, vec![], 10_000);
        unclear.answers = vec![Heard { words: "hm".into(), read: Reply::Unclear, at: 9_000, via: Via::Typed }];
        unclear.asked_again = 1;
        let entries = vec![entry(1, Outcome::Done, vec![2], 100_000), entry(2, Outcome::Undone, vec![7], 100_000), entry(3, Outcome::Done, vec![7], 100_000), unclear, entry(5, Outcome::Cancelled, vec![], 10_000)];
        let events = [back(2, 200, "reopened"), back(7, 50, "reopened"), back(7, 300, "comment")];
        let flagged = flagged(entries, &events);
        assert_eq!(flagged[0].flags, [Flag::RevertedLater], "reopened after it was accepted in words");
        assert_eq!(flagged[1].flags, [Flag::Undone]);
        assert_eq!(flagged[2].flags, Vec::<Flag>::new(), "reopened before, and only commented after");
        assert_eq!(flagged[3].flags, [Flag::AskedAgain, Flag::Ambiguous]);
        assert_eq!(flagged[4].flags, [Flag::Cancelled]);
        let line = week_line(&flagged, 0).unwrap();
        assert_eq!(line, "Decided in the person's words: 5 (3 done) · undone 1 · cancelled 1 · asked again 1 · ambiguous 1 · held 0 · taken back later 1");
        assert_eq!(week_line(&flagged, 1_000_000), None, "nothing this week: the retro as before");
        let text = flagged.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>().join("\n") + "\nnot json\n";
        assert_eq!(parse_log(&text).len(), 5);
    }

    #[test]
    fn a_card_is_what_high_words_raise_when_cards_decide() {
        assert_eq!(card_for(&tickets(Verb::Accept, &[1])), Some((Verdict::Accept, vec![1])));
        assert_eq!(card_for(&tickets(Verb::Push, &[1, 4])), Some((Verdict::Merge, vec![1, 4])));
        assert_eq!(card_for(&tickets(Verb::Idea, &[5])), None);
        assert_eq!(card_for(&[Ask::RunRoutine(1)]), None);
        assert_eq!(card_for(&[Ask::RunRoutine(1), Ask::RunRoutine(2)]), None);
    }
}
