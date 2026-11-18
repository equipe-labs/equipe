//! Design follow-ups: the tickets a designer files from a finding — design
//! QA, a daily review, the critic, a second look, something it couldn't
//! confirm — and how far they may go.
//!
//! Left alone, findings fed themselves: each design QA or critique filed a
//! ticket per polish point, each of those was built, checked across the
//! whole width × theme × text range and given its own design QA, which filed
//! more — chains thirteen tickets deep, most of them S3 polish of polish. So
//! each follow-up keeps where it came from ([`FollowUp`]): the ticket its
//! finding was about (its parent), the work the chain started from (its
//! root) and how many follow-ups deep it is (its generation). Then:
//!
//! - an S1/S2 finding is always a ticket, at once — the quality bar holds;
//! - S3/S4 polish of ordinary work is one ticket per parent: a second
//!   finding about the same work goes into its open follow-up, while nobody
//!   has started it ([`takes_more`]);
//! - S3/S4 found while working a follow-up (generation 2 and on), or with
//!   [`POLISH_CAP`] polish follow-ups already open in the squad, is no new
//!   ticket: it goes into the root's design backlog, an idea the person
//!   (or the PM, putting it to them) starts when they want it;
//! - a follow-up's build gets no designer turn of its own when it is polish,
//!   or a follow-up of a follow-up ([`FollowUp::quiet_qa`]): equipe's
//!   precheck still refuses a new S1/S2, and the daily design review sees it;
//! - a polish follow-up is re-checked where its finding is, not across the
//!   whole range ([`FollowUp::light_check`]).
//!
//! No runtime types here: the runtime keeps the records (`journal`) and
//! files the tickets.

use equipe_protocol::TicketId;

use crate::protocol::{Priority, Ticket, TicketStatus};

/// Polish follow-ups open in a squad at most: more findings go to the backlog.
pub const POLISH_CAP: usize = 6;
/// From this generation on, S3/S4 findings go to the backlog, not a ticket.
pub const BACKLOG_FROM: u32 = 2;

/// How much a finding costs the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Function or data broken, content lost.
    S1,
    /// Layout broken at a major width, a WCAG A/AA failure, a clear difference from the spec.
    S2,
    /// What keeps it from looking finished.
    S3,
    /// A finishing note.
    S4,
}

impl Severity {
    pub const ALL: [Severity; 4] = [Severity::S1, Severity::S2, Severity::S3, Severity::S4];

    pub fn code(self) -> &'static str {
        match self {
            Severity::S1 => "s1",
            Severity::S2 => "s2",
            Severity::S3 => "s3",
            Severity::S4 => "s4",
        }
    }

    /// `S2`, `s2` or ` S2 `.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        Severity::ALL.into_iter().find(|s| s.code() == text)
    }

    /// S1/S2: fixed now, whatever the depth.
    pub fn blocks(self) -> bool {
        matches!(self, Severity::S1 | Severity::S2)
    }

    /// As written on a ticket: `S2`.
    pub fn label(self) -> &'static str {
        match self {
            Severity::S1 => "S1",
            Severity::S2 => "S2",
            Severity::S3 => "S3",
            Severity::S4 => "S4",
        }
    }
}

/// What a follow-up is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Filed from a ticket with no severity: ordinary work split off it.
    Plain,
    /// An S1/S2 finding to fix.
    Fix,
    /// S3/S4 findings of one parent, together.
    Polish,
    /// A root's S3/S4 findings kept for the person: an idea, never started by itself.
    Backlog,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Plain, Kind::Fix, Kind::Polish, Kind::Backlog];

    pub fn code(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Fix => "fix",
            Kind::Polish => "polish",
            Kind::Backlog => "backlog",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Kind::ALL.into_iter().find(|k| k.code() == text)
    }
}

/// Where a follow-up ticket came from, as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUp {
    pub ticket: TicketId,
    /// The ticket its finding is about: the work a design QA checked, the
    /// review it came from, or the ticket its designer was on.
    pub parent: TicketId,
    /// The ordinary work the chain started from.
    pub root: TicketId,
    /// 1 for a follow-up of ordinary work, 2 for a follow-up of that, …
    pub generation: u32,
    pub kind: Kind,
    pub severity: Option<Severity>,
}

impl FollowUp {
    /// Whether its build goes on without a designer turn of its own: polish,
    /// or any follow-up of a follow-up. Design QA stays one level deep.
    pub fn quiet_qa(&self) -> bool {
        matches!(self.kind, Kind::Polish | Kind::Backlog) || self.generation >= BACKLOG_FROM
    }

    /// Whether its hand-in is re-checked where its findings are, not across
    /// the whole width × theme × text range: polish only. An S1/S2 fix is
    /// shown across the whole range.
    pub fn light_check(&self) -> bool {
        matches!(self.kind, Kind::Polish | Kind::Backlog)
    }
}

/// Where a follow-up's chain stands, before a new finding is filed from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lineage {
    pub parent: TicketId,
    pub root: TicketId,
    /// The generation a ticket filed now gets.
    pub generation: u32,
    /// Filed from design QA, a review, or a follow-up's own work: it says its severity.
    pub finding: bool,
}

/// The lineage of a ticket filed about `parent`, whose own record (if it is
/// a follow-up itself) is `of_parent`; `finding` when it is filed from
/// design QA or a review.
pub fn lineage(parent: TicketId, of_parent: Option<&FollowUp>, finding: bool) -> Lineage {
    match of_parent {
        Some(p) => Lineage { parent, root: p.root, generation: p.generation + 1, finding: true },
        None => Lineage { parent, root: parent, generation: 1, finding },
    }
}

/// Why S3/S4 went to the backlog rather than a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Found while working a follow-up.
    Depth,
    /// [`POLISH_CAP`] polish follow-ups are open already.
    Cap,
    /// The meta-work budget (`crate::flow`): polish is never a ticket to run
    /// of its own — in scope it is an item of the change's checklist, out of
    /// it a backlog note for the person.
    Budget,
}

/// What becomes of a ticket a designer files from a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Refused: filed from a finding, it says its severity.
    NeedsSeverity,
    /// An ordinary ticket, kept with its lineage.
    Plain,
    /// A fix ticket (or into the parent's open one).
    Fix,
    /// The parent's polish ticket (or into its open one).
    Polish,
    /// Into the root's design backlog.
    Backlog(Why),
}

impl Place {
    /// The kind of follow-up it makes, once placed.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Place::NeedsSeverity => None,
            Place::Plain => Some(Kind::Plain),
            Place::Fix => Some(Kind::Fix),
            Place::Polish => Some(Kind::Polish),
            Place::Backlog(_) => Some(Kind::Backlog),
        }
    }
}

/// Where a finding filed with `severity` from `lineage` goes, with
/// `open_polish` polish follow-ups open in the squad.
pub fn place(lineage: &Lineage, severity: Option<Severity>, open_polish: usize) -> Place {
    match severity {
        None if lineage.finding => Place::NeedsSeverity,
        None => Place::Plain,
        Some(s) if s.blocks() => Place::Fix,
        Some(_) if lineage.generation >= BACKLOG_FROM => Place::Backlog(Why::Depth),
        Some(_) if open_polish >= POLISH_CAP => Place::Backlog(Why::Cap),
        Some(_) => Place::Polish,
    }
}

/// The priority a follow-up of `kind` gets: fixes come first, polish when it fits.
pub fn priority(kind: Kind) -> Option<Priority> {
    match kind {
        Kind::Fix => Some(Priority::High),
        Kind::Polish | Kind::Backlog => Some(Priority::Low),
        Kind::Plain => None,
    }
}

/// Whether follow-up ticket `t` can take another finding: open, and nobody
/// has started it.
pub fn takes_more(t: &Ticket) -> bool {
    matches!(t.status, TicketStatus::Proposed | TicketStatus::Todo | TicketStatus::Idea) && t.branch.is_none() && t.assignee.is_none()
}

/// `body` with one more finding added: its title, severity and where it came
/// from, then what it says.
pub fn appended(body: &str, title: &str, severity: Option<Severity>, from: TicketId, text: &str) -> String {
    let tag = severity.map(|s| format!("{}, ", s.label())).unwrap_or_default();
    let text = text.trim();
    let item = if text.is_empty() { format!("### {} ({tag}T-{from})", title.trim()) } else { format!("### {} ({tag}T-{from})\n{text}", title.trim()) };
    format!("{}\n\n{item}", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(generation: u32, kind: Kind) -> FollowUp {
        FollowUp { ticket: 9, parent: 4, root: 2, generation, kind, severity: None }
    }

    #[test]
    fn severities_read_either_case_and_only_s1_s2_block() {
        assert_eq!(Severity::parse(" S2 "), Some(Severity::S2));
        assert_eq!(Severity::parse("s4"), Some(Severity::S4));
        assert_eq!(Severity::parse("P1"), None);
        assert!(Severity::S1.blocks() && Severity::S2.blocks() && !Severity::S3.blocks() && !Severity::S4.blocks());
    }

    #[test]
    fn a_follow_up_of_a_follow_up_is_one_deeper_with_the_same_root() {
        let first = lineage(4, None, false);
        assert_eq!(first, Lineage { parent: 4, root: 4, generation: 1, finding: false });
        let from_qa = lineage(4, None, true);
        assert!(from_qa.finding && from_qa.generation == 1);
        let deeper = lineage(9, Some(&up(1, Kind::Polish)), false);
        assert_eq!(deeper, Lineage { parent: 9, root: 2, generation: 2, finding: true }, "working a follow-up is a finding context");
    }

    #[test]
    fn s1_s2_are_always_a_ticket_and_polish_stops_at_the_second_generation() {
        let qa = lineage(4, None, true);
        assert_eq!(place(&qa, None, 0), Place::NeedsSeverity);
        assert_eq!(place(&qa, Some(Severity::S2), POLISH_CAP + 5), Place::Fix, "the cap never holds back a blocker");
        assert_eq!(place(&qa, Some(Severity::S3), 0), Place::Polish);
        let deep = lineage(9, Some(&up(3, Kind::Fix)), false);
        assert_eq!(place(&deep, Some(Severity::S1), 0), Place::Fix);
        assert_eq!(place(&deep, Some(Severity::S3), 0), Place::Backlog(Why::Depth));
        assert_eq!(place(&deep, Some(Severity::S4), 0), Place::Backlog(Why::Depth));
        let plain = lineage(4, None, false);
        assert_eq!(place(&plain, None, POLISH_CAP), Place::Plain, "ordinary work split off a ticket is untouched");
    }

    #[test]
    fn the_squads_cap_sends_more_polish_to_the_backlog() {
        let qa = lineage(4, None, true);
        assert_eq!(place(&qa, Some(Severity::S3), POLISH_CAP - 1), Place::Polish);
        assert_eq!(place(&qa, Some(Severity::S3), POLISH_CAP), Place::Backlog(Why::Cap));
    }

    #[test]
    fn design_qa_stays_one_level_deep_and_polish_is_checked_where_it_is() {
        assert!(!up(1, Kind::Fix).quiet_qa() && !up(1, Kind::Plain).quiet_qa(), "a first-level fix gets its own design QA");
        assert!(up(1, Kind::Polish).quiet_qa() && up(1, Kind::Backlog).quiet_qa());
        assert!(up(2, Kind::Fix).quiet_qa() && up(2, Kind::Plain).quiet_qa(), "a follow-up of a follow-up never does");
        assert!(up(1, Kind::Polish).light_check() && !up(1, Kind::Fix).light_check() && !up(3, Kind::Fix).light_check());
    }

    #[test]
    fn only_an_unstarted_follow_up_takes_more_findings() {
        let mut t = Ticket {
            id: 9,
            title: "Polish".into(),
            body: String::new(),
            status: TicketStatus::Todo,
            owner: Some("frontend".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None, landing: false,
            plan: None,
            model: None,
            flow: None,
            created_by: "designer".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
            kind: crate::protocol::TicketKind::Code,
        };
        assert!(takes_more(&t));
        t.assignee = Some("frontend".into());
        assert!(!takes_more(&t));
        t.assignee = None;
        t.branch = Some("equipe/frontend-t9".into());
        assert!(!takes_more(&t));
        t.branch = None;
        t.status = TicketStatus::Review;
        assert!(!takes_more(&t));
    }

    #[test]
    fn a_finding_is_added_under_its_title_with_where_it_came_from() {
        let body = appended("Polish of T-4:\n\n### Gap (S3, T-4)\n24 → 32", "Label wraps", Some(Severity::S3), 4, "At 390, text 200%.\n");
        assert!(body.ends_with("### Label wraps (S3, T-4)\nAt 390, text 200%."), "{body}");
        assert_eq!(appended("x", "Bare", None, 7, " "), "x\n\n### Bare (T-7)");
    }
}
