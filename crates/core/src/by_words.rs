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
use equipe_protocol::by_words::{self, Entry, Flag, Kind, Outcome, Quote, Reply, Said, Stage, Tier, Tongues, own_words};
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
    /// A member's request for permission, once. "Always" is its card's alone.
    Approval(u64),
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
        Ask::Approval(id) => w.approvals.iter().find(|a| a.id == *id).map_or(Kind::Blocked, approval_kind),
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
        Ask::Approval(id) => w.approvals.iter().any(|a| a.id == *id),
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
        Ask::Approval(id) => w.approvals.iter().find(|a| a.id == *id).and_then(|a| a.ticket).into_iter().collect(),
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

/// The person's words, read as plainly asking for something that waits:
/// every request they fit — none, one, or several to choose from — read in
/// the person's languages (`t`). A question, a no, an "if", a "later" —
/// anything less plain — is left to the member they said it to, and so are
/// words naming nothing that waits. An approval word alone ("approve",
/// "allow") starts something only when the message is little more than it,
/// or names what it approves: its member, its ticket, a word of its command.
pub fn read(text: &str, w: &Waiting, t: &Tongues) -> Vec<Ask> {
    let words = own_words(text);
    let lower = words.trim().to_lowercase();
    if lower.is_empty() || lower.chars().count() > by_words::MOST_CHARS || lower.starts_with('/') || lower.starts_with('@') {
        return Vec::new();
    }
    if lower.contains(['?', '？']) || t.written(&lower, "match.unsure") || by_words::says_no(t, &lower) {
        return Vec::new();
    }
    let has = |key: &str| t.any(&lower, key);
    let named = named_tickets(&lower);
    // Tickets: what the words do to them.
    let verb = if t.written(&lower, "match.merge") || has("match.push") {
        Some(if has("match.push") { Verb::Push } else { Verb::Merge })
    } else if has("match.branch") {
        Some(Verb::KeepOnBranch)
    } else if has("match.idea") {
        Some(Verb::Idea)
    } else if !named.is_empty() && has("match.take_on") {
        Some(Verb::TakeOn)
    } else if t.written(&lower, "match.accept") {
        Some(Verb::Accept)
    } else {
        None
    };
    if let Some(verb) = verb {
        let ids = if named.is_empty() {
            match verb {
                Verb::Accept | Verb::Merge | Verb::Push if says_all(&lower, t) => all_of(w.tickets, if verb == Verb::Accept { Verdict::Accept } else { Verdict::Merge }),
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
    // A routine, by its title: run once, or the PM's turned on.
    let titled = |title: &str| {
        let title = title.trim().to_lowercase();
        title.chars().count() >= 2 && lower.contains(&title)
    };
    if named.is_empty() && has("match.run") {
        let runs: Vec<Ask> = w.routines.iter().filter(|r| titled(&r.title)).map(|r| Ask::RunRoutine(r.id)).collect();
        if !runs.is_empty() {
            return runs;
        }
    }
    // "Always": a lasting rule is its card's alone.
    if !has("match.allow") || has("match.always") {
        return Vec::new();
    }
    let titled_routines: Vec<Ask> = w.routine_proposals.iter().filter(|p| titled(&p.routine.title)).map(|p| Ask::ApproveRoutine(p.id)).collect();
    if named.is_empty() && !titled_routines.is_empty() {
        return titled_routines;
    }
    // What waits, each with what names it: a member, tickets, the words of its command.
    struct Waits<'a> {
        who: Option<&'a str>,
        tickets: Vec<TicketId>,
        action: Option<&'a str>,
        ask: Ask,
    }
    let mut all: Vec<Waits> = Vec::new();
    for a in w.approvals.iter().filter(|a| approval_kind(a).reach() == by_words::Reach::Words) {
        all.push(Waits { who: Some(&a.member), tickets: a.ticket.into_iter().collect(), action: Some(&a.action), ask: Ask::Approval(a.id) });
    }
    all.extend(w.accepts.iter().filter(|c| !c.from_words()).map(|c| Waits { who: Some(&c.by), tickets: c.tickets.clone(), action: None, ask: Ask::Card(c.id) }));
    all.extend(w.routine_proposals.iter().map(|p| Waits { who: Some(&p.by), tickets: Vec::new(), action: None, ask: Ask::ApproveRoutine(p.id) }));
    all.extend(w.team_changes.iter().map(|c| Waits { who: Some(&c.by), tickets: c.unblocks.clone(), action: None, ask: Ask::Team(c.id) }));
    all.extend(w.engine_proposals.iter().filter(|p| !p.applied).map(|p| Waits { who: None, tickets: Vec::new(), action: None, ask: Ask::Engine(p.id) }));
    let plain = |w: &str| t.written(w, "match.allow") || t.words("match.yes").contains(&w) || t.words("match.yes_with").contains(&w);
    let tokens = by_words::tokens(&lower);
    let named_words: Vec<&String> = tokens.iter().filter(|w| w.chars().count() >= 4 && !plain(w)).collect();
    let names = |x: &Waits| {
        x.who.is_some_and(|m| holds_name(&lower, m))
            || named.iter().any(|t| x.tickets.contains(t))
            || x.action.is_some_and(|a| {
                let a = a.to_lowercase();
                named_words.iter().any(|w| a.contains(w.as_str()))
            })
    };
    let to: Vec<Ask> = all.iter().filter(|x| names(x)).map(|x| x.ask.clone()).collect();
    if !to.is_empty() {
        return to;
    }
    // Nothing named: only the approval word itself, on its own, is asking.
    let bare = named.is_empty() && !lower.contains('\n') && tokens.len() <= by_words::SPOKEN_YES_WORDS && tokens.iter().all(|w| plain(w));
    if bare { all.into_iter().map(|x| x.ask).collect() } else { Vec::new() }
}

/// "all", "everything" and their kin, in the person's languages (Korean writes "accept all" as one word too).
fn says_all(lower: &str, t: &Tongues) -> bool {
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '~')).filter(|w| !w.is_empty()).collect();
    let starts = t.words("match.all_starts");
    let whole = t.words("match.all_words");
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
        Ask::Approval(id) => vec![Command::Decide { squad, approval: *id, decision: Decision::Allow, words: None, said }],
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

/// The most of a title a read-back names.
const TITLE_CHARS: usize = 80;

/// A title a member gave, as a read-back names it: on one line, clipped.
pub fn title(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > TITLE_CHARS { format!("{}…", flat.chars().take(TITLE_CHARS).collect::<String>()) } else { flat }
}

fn ticket_line(lang: Lang, t: &Ticket, verb: Option<Verb>) -> String {
    let words = core(lang);
    let base = words.fill("words.item.ticket", &[("id", t.id.to_string()), ("title", title(&t.title))]);
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

/// What equipe reads back: its own line on what will happen (the action
/// and its target — ticket, branch, command, recipient — from the request's
/// fields), each thing it does, and, apart, what members wrote about it,
/// quoted as theirs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadBack {
    pub head: String,
    pub items: Vec<String>,
    pub quotes: Vec<Quote>,
}

/// What equipe reads back for `ask`. Never a member's words in equipe's
/// voice: a reason, a note, what a member asks to do in the person's name —
/// amounts and recipients as it wrote them — are [`Quote`]s.
pub fn read_back(lang: Lang, ask: &Ask, w: &Waiting) -> ReadBack {
    let words = core(lang);
    let quote = |by: &str, text: &str| (!text.trim().is_empty()).then(|| Quote::of(by, text));
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
            ReadBack { head: words.fill(key, &[("tickets", list(tickets))]), items: found.iter().map(|t| ticket_line(lang, t, Some(*verb))).collect(), quotes: Vec::new() }
        }
        Ask::RunRoutine(id) => {
            let name = w.routines.iter().find(|r| r.id == *id).map(|r| title(&r.title)).unwrap_or_default();
            ReadBack { head: words.fill("words.do.run_routine", &[("title", name)]), ..ReadBack::default() }
        }
        Ask::ApproveRoutine(id) => match w.routine_proposals.iter().find(|p| p.id == *id) {
            Some(p) => ReadBack {
                head: words.fill("words.do.approve_routine", &[("title", title(&p.routine.title)), ("when", schedule(lang, p.routine.when))]),
                items: vec![words.fill("words.item.proposed", &[("member", p.by.clone())])],
                quotes: quote(&p.by, &p.reason).into_iter().collect(),
            },
            None => ReadBack::default(),
        },
        Ask::Team(id) => match w.team_changes.iter().find(|c| c.id == *id) {
            Some(c) => {
                let head = match c.change {
                    TeamMove::Add => words.fill("words.do.team_add", &[("n", c.count.to_string()), ("role", c.role.clone())]),
                    TeamMove::Remove => words.fill("words.do.team_remove", &[("member", c.member.clone().unwrap_or_else(|| c.role.clone()))]),
                };
                ReadBack { head, items: vec![words.fill("words.item.proposed", &[("member", c.by.clone())])], quotes: quote(&c.by, &c.reason).into_iter().collect() }
            }
            None => ReadBack::default(),
        },
        Ask::Engine(id) => match w.engine_proposals.iter().find(|p| p.id == *id) {
            Some(p) => {
                let members: Vec<String> = p.moves.iter().filter(|m| !m.in_place()).map(|m| m.member.clone()).collect();
                let to = p.to().map(|e| e.label().to_owned()).unwrap_or_default();
                ReadBack { head: words.fill("words.do.engine", &[("members", members.join(", ")), ("engine", to)]), ..ReadBack::default() }
            }
            None => ReadBack::default(),
        },
        Ask::Card(id) => match w.accepts.iter().find(|c| c.id == *id) {
            Some(card) => {
                let verb = match card.verdict {
                    Verdict::Accept => Verb::Accept,
                    Verdict::Merge => Verb::Merge,
                    Verdict::Changes => {
                        return ReadBack {
                            head: words.fill("words.do.send_back", &[("tickets", list(&card.tickets))]),
                            items: vec![words.fill("words.item.proposed", &[("member", card.by.clone())])],
                            quotes: quote(&card.by, &card.note).into_iter().collect(),
                        };
                    }
                };
                let mut back = read_back(lang, &Ask::Tickets { verb, tickets: card.tickets.clone() }, w);
                back.items.push(words.fill("words.item.proposed", &[("member", card.by.clone())]));
                back
            }
            None => ReadBack::default(),
        },
        Ask::Approval(id) => {
            let Some(a) = w.approvals.iter().find(|a| a.id == *id) else { return ReadBack::default() };
            let kind = approval_kind(a);
            let (head, mut quotes) = match kind {
                // What it asks to do in the person's name is the member's own sentence: quoted.
                Kind::ActInName => (words.fill("words.do.act_in_name", &[("member", a.member.clone())]), quote(&a.member, &a.action).into_iter().collect::<Vec<_>>()),
                _ => {
                    let key = match kind {
                        Kind::SendAsPerson => "words.do.send",
                        Kind::SignInWithLogin => "words.do.sign_in",
                        Kind::WidenSetting => "words.do.setting",
                        Kind::TouchSecrets => "words.do.secrets",
                        Kind::AllowOutward => "words.do.outward",
                        _ => "words.do.allow",
                    };
                    (words.fill(key, &[("member", a.member.clone()), ("action", a.action.clone())]), Vec::new())
                }
            };
            let items = a.ticket.map(|t| words.fill("words.item.on", &[("ticket", format!("T-{t}"))])).into_iter().collect();
            quotes.extend(a.reason.as_deref().and_then(|why| quote(&a.member, why)));
            ReadBack { head, items, quotes }
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
    asks.iter().map(|a| read_back(lang, a, w).head).collect()
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
/// choosing among `count`, asked again `asked_again` times — read in the
/// person's languages (`t`), and `strict`ly when said aloud or dictated
/// about something high-risk — and how it was read, for the log.
/// Deterministic: the word lists, never a model.
///
/// Read back or choosing: a clear yes (or a number) does it, any no holds
/// it on its card, a yes with more said is asked again — and anything else
/// goes on to the member as it is, the request still waiting until it lapses
/// to its card. Never swallowed.
pub fn answer(t: &Tongues, stage: Stage, tier: Tier, strict: bool, text: &str, count: usize, asked_again: u32) -> (Step, Reply) {
    let words = own_words(text);
    let long = words.chars().count() > by_words::MOST_CHARS || words.lines().count() > 2;
    let again = |read: Reply| if asked_again + 1 > by_words::ASK_AGAIN { (Step::Close { outcome: Outcome::Held, pass: false }, read) } else { (Step::Again, read) };
    match stage {
        Stage::Undo | Stage::Countdown => {
            if by_words::stops(t, text) {
                let outcome = if stage == Stage::Undo { Outcome::Undone } else { Outcome::Cancelled };
                (Step::Close { outcome, pass: false }, if stage == Stage::Undo { Reply::Undo } else { Reply::Cancel })
            } else {
                (Step::Pass, Reply::Other)
            }
        }
        Stage::ReadBack | Stage::Which => {
            if long {
                return (Step::Pass, Reply::Other);
            }
            let read = by_words::reply(t, text, strict && tier != Tier::Low);
            if read == Reply::No {
                return (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No);
            }
            if stage == Stage::Which {
                return match (by_words::picked(t, text, count), read) {
                    (Some(n), _) => (Step::Choose(n), Reply::Pick),
                    (None, Reply::Other) => (Step::Pass, Reply::Other),
                    (None, _) => again(Reply::Unclear),
                };
            }
            match read {
                Reply::Yes if tier == Tier::Outward => (Step::Countdown, Reply::Yes),
                Reply::Yes => (Step::Run, Reply::Yes),
                Reply::Other => (Step::Pass, Reply::Other),
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

/// The entries kept: those closed within `days` of `now_ms` — the journal's own keeping.
pub fn prune(entries: Vec<Entry>, now_ms: u64, days: u64) -> Vec<Entry> {
    let since = now_ms.saturating_sub(days * 24 * 60 * 60 * 1000);
    entries.into_iter().filter(|e| e.closed_at.max(e.said_at) >= since).collect()
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

/// Where what waits in words is kept while it waits: what a daemon that
/// stopped left waiting is logged as dropped when it starts again.
pub fn live_path(home: &Path, squad: &str) -> std::path::PathBuf {
    crate::journal::squad_dir(home, squad).join("words-live.jsonl")
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
        "Decided in the person's words: {} ({} done) · undone {} · cancelled {} · asked again {} · ambiguous {} · held {} · taken back later {} · dropped by a restart {}",
        week.len(),
        done,
        count(Flag::Undone),
        count(Flag::Cancelled),
        count(Flag::AskedAgain),
        count(Flag::Ambiguous),
        count(Flag::Held),
        count(Flag::RevertedLater),
        count(Flag::Dropped)
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

    fn tongue(code: &str) -> Tongues {
        Tongues::of(&[code])
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
        // A lasting widening is its card's in every mode.
        assert_eq!(Kind::AllowAlways.reach(), Reach::Card);
        assert_eq!(Kind::WidenSetting.reach(), Reach::Card);
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
        assert_eq!(t(Ask::Approval(10)), Tier::High);
        assert_eq!(t(Ask::Approval(11)), Tier::Outward);
        assert_eq!(t(Ask::Approval(12)), Tier::Outward, "paying");
        assert_eq!(t(Ask::Card(20)), Tier::High);
        assert_eq!(t(Ask::Approval(99)), Tier::Outward, "gone: as refused");
    }

    #[test]
    fn plain_words_name_what_waits_and_anything_less_plain_is_left_to_the_member() {
        let f = Fixture::new();
        let w = f.waiting();
        let (ko, en) = (tongue("ko"), tongue("en"));
        let read_ko = |text: &str| read(text, &w, &ko);
        let read_en = |text: &str| read(text, &w, &en);
        // Accepting, the way a card was raised before.
        assert_eq!(read_ko("T-2 수락"), tickets(Verb::Accept, &[2]));
        assert_eq!(read_en("accept T-2 and T-7"), tickets(Verb::Accept, &[2, 7]));
        assert_eq!(read_ko("전부다 수락할게."), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read_ko("다수락해줘"), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read_en("Accept them all!"), tickets(Verb::Accept, &[1, 2, 7]));
        assert_eq!(read_ko("T-1 머지"), tickets(Verb::Merge, &[1]));
        assert_eq!(read_en("merge T-4"), tickets(Verb::Merge, &[4]));
        assert_eq!(read_en("merge T-1 and push"), tickets(Verb::Push, &[1]));
        assert_eq!(read_ko("T-1 푸시까지"), tickets(Verb::Push, &[1]));
        assert_eq!(read_ko("전부 머지까지 해줘"), tickets(Verb::Merge, &[1, 4]), "to merge, accepted code off main too; research has nothing to merge");
        assert_eq!(read_en("leave T-4 on its branch"), tickets(Verb::KeepOnBranch, &[4]));
        assert_eq!(read_ko("T-5 아이디어로 남겨"), tickets(Verb::Idea, &[5]));
        assert_eq!(read_en("keep T-5 as an idea"), tickets(Verb::Idea, &[5]));
        assert_eq!(read_en("take on T-6"), tickets(Verb::TakeOn, &[6]));
        assert_eq!(read_ko("T-5 진행"), tickets(Verb::TakeOn, &[5]));
        assert_eq!(read_en("run the weekly report"), vec![Ask::RunRoutine(1)]);
        assert_eq!(read_ko("주간 리포트 한 번 돌려 줘"), vec![Ask::RunRoutine(2)]);
        // Named but not fit, or nothing named: nothing.
        assert_eq!(read_ko("T-3 수락"), vec![], "being done");
        assert_eq!(read_ko("T-1, T-3, T-99 수락"), tickets(Verb::Accept, &[1]));
        assert_eq!(read_ko("수락할게"), vec![]);
        assert_eq!(read_en("run it"), vec![], "no routine named");
        for unsure in [
            "전부 수락할까?",
            "T-1 수락하지 마",
            "아직 수락하지 말아줘",
            "테스트 통과하면 다 수락할게",
            "수락 버튼 어디 있어",
            "T-1 수락 버튼이 안 눌려요",
            "이건 반려",
            "T-1 어때",
            "/accept T-1",
            "@pm accept T-1",
            "다 했어요",
            "오늘 한 일 정리해서 내일 아침에 다시 보여줘. 그리고 T-1은 리뷰가 끝나면 수락할 생각이에요, 문제 없으면요",
        ] {
            assert_eq!(read_ko(unsure), vec![], "{unsure}");
        }
        for unsure in ["don't accept T-1", "accept all after the tests", "don't merge T-1", "not yet, merge T-1 later"] {
            assert_eq!(read_en(unsure), vec![], "{unsure}");
        }
        // Quoted or fenced words are never the person's.
        assert_eq!(read_en("> merge T-1\nok, thanks"), vec![]);
        assert_eq!(read_en("<!-- reply 1 -->\n> accept T-2\n<!-- reply -->\nlooks fine"), vec![]);
        assert_eq!(read_en("```\nmerge T-1\n```"), vec![]);
    }

    #[test]
    fn plain_words_are_read_in_each_language() {
        let f = Fixture::new();
        let w = f.waiting();
        for (code, text, verb, ids) in [
            ("en", "T-2 accept", Verb::Accept, vec![2]),
            ("ja", "T-2 承認して", Verb::Accept, vec![2]),
            ("ja", "全部承認", Verb::Accept, vec![1, 2, 7]),
            ("ja", "T-1 マージして", Verb::Merge, vec![1]),
            ("zh-Hans", "接受 T-2", Verb::Accept, vec![2]),
            ("zh-Hans", "全部接受", Verb::Accept, vec![1, 2, 7]),
            ("zh-Hans", "T-1 合并", Verb::Merge, vec![1]),
            ("zh-Hant", "T-1 合併", Verb::Merge, vec![1]),
            ("es", "aprueba T-2", Verb::Accept, vec![2]),
            ("es", "acepta todo", Verb::Accept, vec![1, 2, 7]),
            ("es", "fusiona T-1", Verb::Merge, vec![1]),
            ("fr", "T-2 validé", Verb::Accept, vec![2]),
            ("fr", "accepte tout", Verb::Accept, vec![1, 2, 7]),
            ("fr", "fusionne T-1", Verb::Merge, vec![1]),
            ("de", "T-2 genehmigen", Verb::Accept, vec![2]),
            ("de", "alles akzeptieren", Verb::Accept, vec![1, 2, 7]),
            ("de", "T-1 zusammenführen", Verb::Merge, vec![1]),
        ] {
            assert_eq!(read(text, &w, &tongue(code)), tickets(verb, &ids), "{code}: {text}");
        }
        for (code, unsure) in [("ja", "まだ承認しないで"), ("zh-Hans", "不要接受 T-1"), ("zh-Hans", "先别合并"), ("es", "no aceptes T-1"), ("fr", "n'accepte pas T-1"), ("de", "T-1 nicht akzeptieren"), ("de", "wenn die Tests laufen, alles akzeptieren")] {
            assert_eq!(read(unsure, &w, &tongue(code)), vec![], "{code}: {unsure}");
        }
        // Another language's words are not read: Japanese accepting, read as Korean and English only.
        assert_eq!(read("T-2 承認して", &w, &tongue("ko")), vec![]);
    }

    #[test]
    fn an_approval_word_starts_something_only_alone_or_naming_what_it_approves() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(10, "backend", "Bash", "npm test"), approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49")];
        f.approvals[1].ticket = Some(4);
        f.accepts = vec![
            AcceptProposal { id: 20, verdict: Verdict::Accept, tickets: vec![1], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 },
            AcceptProposal { id: 21, verdict: Verdict::Accept, tickets: vec![2], note: String::new(), summary: String::new(), by: FROM_WORDS.into(), session: 1, proposed_at: 0 },
        ];
        f.proposals = vec![RoutineProposal { id: 30, routine: routine(0, "Dependency check"), reason: "weekly".into(), by: "pm".into(), session: 1, proposed_at: 0 }];
        let w = f.waiting();
        let (ko, en) = (tongue("ko"), tongue("en"));
        // Named: by its member, its ticket, a word of its command, its title.
        assert_eq!(read("allow backend", &w, &en), vec![Ask::Approval(10)]);
        assert_eq!(read("frontend 허용", &w, &ko), vec![Ask::Approval(11)]);
        assert_eq!(read("allow the push", &w, &en), vec![Ask::Approval(11)]);
        assert_eq!(read("T-4 승인", &w, &ko), vec![Ask::Approval(11)], "T-4 isn't for accepting: its request is");
        assert_eq!(read("approve the dependency check", &w, &en), vec![Ask::ApproveRoutine(30)]);
        // Alone, everything waiting is put to the person to choose. A card raised from their own words is theirs to click.
        let every = vec![Ask::Approval(10), Ask::Approval(11), Ask::Approval(12), Ask::Card(20), Ask::ApproveRoutine(30)];
        assert_eq!(read("승인", &w, &ko), every);
        assert_eq!(read("승인해 줘", &w, &ko), every);
        assert_eq!(read("allow it", &w, &en), every);
        // An approval word in a sentence about something else starts nothing.
        assert_eq!(read("그 계획 승인", &w, &ko), vec![]);
        assert_eq!(read("approve the plan and write the tickets", &w, &en), vec![]);
        // "Always" is a card's, in every mode.
        assert_eq!(read("always allow backend", &w, &en), vec![]);
        assert_eq!(read("앞으로 항상 허용", &w, &ko), vec![]);
        assert_eq!(read("don't allow backend", &w, &en), vec![]);
        assert_eq!(read("allow backend?", &w, &en), vec![]);
        let empty = Fixture { approvals: vec![], accepts: vec![], proposals: vec![], ..Fixture::new() };
        assert_eq!(read("allow it", &empty.waiting(), &en), vec![]);
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
        assert_eq!(run(Ask::Approval(10)), [Command::Decide { squad: s(), approval: 10, decision: Decision::Allow, words: None, said: Some(said()) }], "once — never always");
        assert_eq!(
            run(Ask::Card(20)),
            [Command::ConfirmAccept { squad: s(), id: 20, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Merge) }, Confirmed::only(2)], said: Some(said()) }],
            "merged where there is code; research goes into the folder"
        );
        assert_eq!(run(Ask::ApproveRoutine(30)), [Command::ApproveRoutine { squad: s(), id: 30 }]);
        assert_eq!(run(Ask::Team(40)), [Command::ApproveTeamChange { squad: s(), id: 40 }]);
        assert_eq!(run(Ask::Engine(50)), [Command::ApplyEngineProposal { squad: s(), id: 50 }]);
        for ask in [Ask::Tickets { verb: Verb::Push, tickets: vec![1, 2, 4] }, Ask::Card(20), Ask::Approval(10)] {
            for c in run(ask) {
                assert!(matches!(c, Command::Accept { .. } | Command::Merge { .. } | Command::ConfirmAccept { .. } | Command::Decide { decision: Decision::Allow, .. }), "{c:?}");
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
        assert!(!still_waits(&Ask::Approval(10), &f.waiting()));
        assert!(still_waits(&Ask::RunRoutine(1), &f.waiting()));
    }

    #[test]
    fn the_read_back_says_the_action_and_its_target_and_quotes_what_members_wrote_apart() {
        let mut f = Fixture::new();
        f.approvals = vec![approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49.00\n a month to Vercel")];
        f.tickets[0].title = "A title\nthat a member wrote long and on two lines, going on and on past where a name should stop".into();
        let w = f.waiting();
        let back = read_back(Lang::En, &Ask::Tickets { verb: Verb::Push, tickets: vec![1] }, &w);
        assert_eq!(back.head, "Merge T-1 into main and push main to origin");
        assert_eq!(back.items, ["T-1 “A title that a member wrote long and on two lines, going on and on past where a …” — equipe/t-1 → main of web"]);
        assert!(back.quotes.is_empty());
        let back = read_back(Lang::Ko, &Ask::Tickets { verb: Verb::Merge, tickets: vec![2] }, &w);
        assert_eq!(back.head, "T-2를 이 Mac에서 main에 머지");
        let back = read_back(Lang::En, &Ask::Approval(11), &w);
        assert_eq!(back.head, "Let frontend reach outside the squad, once: git push origin main");
        assert_eq!(back.items, ["On T-3"]);
        assert_eq!(back.quotes, [Quote { by: "frontend".into(), text: "to see the tests".into() }], "the member's reason, as theirs");
        // What a member asks to do in the person's name is never in equipe's voice: quoted, flat.
        let back = read_back(Lang::En, &Ask::Approval(12), &w);
        assert_eq!(back.head, "Let pm act in your name — what it asks, in its words, below");
        assert!(!back.head.contains("49"));
        assert_eq!(back.quotes[0], Quote { by: "pm".into(), text: "Pay $49.00 a month to Vercel".into() });
        assert_eq!(read_back(Lang::En, &Ask::RunRoutine(1), &w).head, "Run the routine “Weekly report” once, now");
        assert_eq!(ask_line(Lang::En, Tier::Outward), "Say yes and it goes in 10 s — say cancel to stop it then.");
        assert_eq!(ask_line(Lang::En, Tier::High), "Say yes to go ahead, or no to leave it on its card.");
    }

    #[test]
    fn a_read_back_runs_only_on_a_clear_yes_and_anything_else_goes_on_to_the_member() {
        let (en, ko) = (tongue("en"), tongue("ko"));
        let rb = |t: &Tongues, text: &str, tier: Tier| answer(t, Stage::ReadBack, tier, false, text, 0, 0);
        assert_eq!(rb(&en, "yes", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb(&ko, "ㄱㄱ", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb(&ko, "고고", Tier::High), (Step::Run, Reply::Yes));
        assert_eq!(rb(&en, "ok", Tier::Outward), (Step::Countdown, Reply::Yes), "outward: counts down first");
        assert_eq!(rb(&en, "no", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(rb(&ko, "아니 하지 마", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(rb(&en, "not yet", Tier::Outward), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        // A yes with more said: asked again — too often, held.
        assert_eq!(rb(&en, "yes?", Tier::High), (Step::Again, Reply::Unclear));
        assert_eq!(rb(&en, "yes and fix the header", Tier::High), (Step::Again, Reply::Unclear));
        assert_eq!(answer(&en, Stage::ReadBack, Tier::High, false, "yes?", 0, by_words::ASK_AGAIN), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::Unclear));
        // Anything else is the member's, the read-back still waiting: never swallowed.
        assert_eq!(rb(&ko, "음", Tier::High), (Step::Pass, Reply::Other));
        assert_eq!(rb(&en, "fix the header first", Tier::High), (Step::Pass, Reply::Other));
        assert_eq!(rb(&en, "> yes", Tier::High), (Step::Pass, Reply::Other), "a quoted yes is not theirs");
        let long = "yes, and first fix the header on the pricing page and then look at the footer links again please";
        assert_eq!(rb(&en, long, Tier::High), (Step::Pass, Reply::Other));
        // Choosing: a number; a yes picks nothing; a no holds; the rest goes on.
        assert_eq!(answer(&en, Stage::Which, Tier::High, false, "2", 3, 0), (Step::Choose(2), Reply::Pick));
        assert_eq!(answer(&en, Stage::Which, Tier::High, false, "yes", 3, 0), (Step::Again, Reply::Unclear));
        assert_eq!(answer(&en, Stage::Which, Tier::High, false, "none of them, no", 3, 0), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
        assert_eq!(answer(&en, Stage::Which, Tier::High, false, "fix the header", 3, 0), (Step::Pass, Reply::Other));
    }

    #[test]
    fn a_spoken_or_dictated_yes_to_something_high_must_be_short_and_on_its_own() {
        let (en, zh) = (tongue("en"), tongue("zh-Hans"));
        let spoken = |t: &Tongues, text: &str, tier: Tier| answer(t, Stage::ReadBack, tier, true, text, 0, 0).0;
        assert_eq!(spoken(&en, "yes", Tier::High), Step::Run);
        assert_eq!(spoken(&en, "yes go ahead please now", Tier::High), Step::Again, "too long");
        assert_eq!(answer(&en, Stage::ReadBack, Tier::High, false, "yes go ahead please now", 0, 0).0, Step::Run, "typed, plain enough");
        assert_eq!(spoken(&zh, "没问题", Tier::Outward), Step::Again, "a negation's letters, said aloud");
        assert_eq!(spoken(&en, "ok", Tier::Outward), Step::Countdown);
    }

    #[test]
    fn undo_and_cancel_windows_stop_on_any_no_and_let_the_rest_through() {
        let (en, ko) = (tongue("en"), tongue("ko"));
        assert_eq!(answer(&en, Stage::Undo, Tier::Low, false, "undo", 0, 0), (Step::Close { outcome: Outcome::Undone, pass: false }, Reply::Undo));
        assert_eq!(answer(&ko, Stage::Undo, Tier::Low, false, "되돌려", 0, 0).0, Step::Close { outcome: Outcome::Undone, pass: false });
        assert_eq!(answer(&en, Stage::Undo, Tier::Low, false, "and fix the header", 0, 0), (Step::Pass, Reply::Other));
        assert_eq!(answer(&en, Stage::Countdown, Tier::Outward, false, "cancel", 0, 0), (Step::Close { outcome: Outcome::Cancelled, pass: false }, Reply::Cancel));
        assert_eq!(answer(&ko, Stage::Countdown, Tier::Outward, false, "잠깐", 0, 0).0, Step::Close { outcome: Outcome::Cancelled, pass: false });
        assert_eq!(answer(&en, Stage::Countdown, Tier::Outward, false, "ok", 0, 0), (Step::Pass, Reply::Other));
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
            quotes: vec![],
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
    fn the_log_flags_what_was_undone_cancelled_unclear_dropped_or_taken_back_later() {
        let back = |ticket: TicketId, at: u64, kind: &str| (ticket, TicketEntry { at, by: "you".into(), kind: kind.into(), text: String::new() });
        let mut unclear = entry(4, Outcome::Done, vec![], 10_000);
        unclear.answers = vec![Heard { words: "yes?".into(), read: Reply::Unclear, at: 9_000, via: Via::Typed }];
        unclear.asked_again = 1;
        let entries = vec![entry(1, Outcome::Done, vec![2], 100_000), entry(2, Outcome::Undone, vec![7], 100_000), entry(3, Outcome::Done, vec![7], 100_000), unclear, entry(5, Outcome::Cancelled, vec![], 10_000), entry(6, Outcome::Dropped, vec![], 10_000)];
        let events = [back(2, 200, "reopened"), back(7, 50, "reopened"), back(7, 300, "comment")];
        let flagged = flagged(entries, &events);
        assert_eq!(flagged[0].flags, [Flag::RevertedLater], "reopened after it was accepted in words");
        assert_eq!(flagged[1].flags, [Flag::Undone]);
        assert_eq!(flagged[2].flags, Vec::<Flag>::new(), "reopened before, and only commented after");
        assert_eq!(flagged[3].flags, [Flag::AskedAgain, Flag::Ambiguous]);
        assert_eq!(flagged[4].flags, [Flag::Cancelled]);
        assert_eq!(flagged[5].flags, [Flag::Dropped]);
        let line = week_line(&flagged, 0).unwrap();
        assert_eq!(line, "Decided in the person's words: 6 (3 done) · undone 1 · cancelled 1 · asked again 1 · ambiguous 1 · held 0 · taken back later 1 · dropped by a restart 1");
        assert_eq!(week_line(&flagged, 1_000_000), None, "nothing this week: the retro as before");
        let text = flagged.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>().join("\n") + "\nnot json\n";
        assert_eq!(parse_log(&text).len(), 6);
    }

    #[test]
    fn the_log_keeps_what_the_journal_keeps() {
        let day = 24 * 60 * 60 * 1000;
        let now = 200 * day;
        let kept = prune(vec![entry(1, Outcome::Done, vec![], now - 91 * day), entry(2, Outcome::Done, vec![], now - 89 * day), entry(3, Outcome::Held, vec![], now)], now, 90);
        assert_eq!(kept.iter().map(|e| e.id).collect::<Vec<_>>(), [2, 3]);
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
