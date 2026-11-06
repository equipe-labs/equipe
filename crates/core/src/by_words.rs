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
    // "Allow it", "승인": whatever waits — narrowed to a member, when the words name one.
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

/// "all", "everything" and their kin, in every language (`다수락` in one word).
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
mod tests;
