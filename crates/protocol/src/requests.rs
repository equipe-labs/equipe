//! The person's requests: what they asked a member for, in their own words,
//! and every ticket that came of it — so one line says where it all stands
//! and nobody has to ask about each ticket in turn.
//!
//! A request is born when the person asks a member for something in a
//! conversation (or puts a ticket on the board themselves). A ticket put on
//! the board in answer joins it — named by the member whenever more than one
//! request is open there; tickets made while working on its tickets —
//! follow-ups, checks, the next step a member files — join it by their
//! lineage. Only a work ticket covers it: one left uncovered is put to the
//! PM, and it closes only when its work is done or a member closes it with
//! a reason the person sees. The daemon works out where each stands
//! (`equipe_core::requests`) and tells the screens with
//! [`crate::Event::RequestChanged`]; a screen only draws it.

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId, TicketStatus};

pub type RequestId = u64;

/// How a ticket came to serve a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// Put on the board in answer to the person's words — or by the person.
    Asked,
    /// Filed by a member while it worked `parent`, a ticket of the request.
    Made,
    /// A design follow-up of `parent`.
    FollowUp,
    /// equipe's own check of `parent` (its design QA): shown in the request's
    /// timeline, never in its status, never news.
    Check,
    /// Put with the request by a member's word (`request` on its ticket).
    Linked,
}

impl Link {
    pub fn code(self) -> &'static str {
        match self {
            Link::Asked => "asked",
            Link::Made => "made",
            Link::FollowUp => "follow_up",
            Link::Check => "check",
            Link::Linked => "linked",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [Link::Asked, Link::Made, Link::FollowUp, Link::Check, Link::Linked].into_iter().find(|l| l.code() == code)
    }
}

/// One ticket of a request, and the ticket of the request it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestTicket {
    pub ticket: TicketId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TicketId>,
    pub link: Link,
}

/// Where one part of a request stands — a ticket asked for, with every
/// ticket that came of it. What waits on the person is said apart
/// ([`Status::waits`]), so a part reads as how far its work got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing started yet: next when its member is free.
    Queued,
    /// A member is on it.
    Working,
    /// Handed in, being reviewed.
    Review,
    /// Its run stopped and only the person brings it back.
    Blocked,
    /// Nothing done yet, and what is open waits on the person.
    Waiting,
    /// Accepted, but its work isn't on the main branch yet.
    Accepted,
    /// Its work is on the main branch, and the project goes live by a deploy
    /// equipe hasn't seen succeed for it yet ([`Part::not_live`] says why
    /// when one failed). Not finished: pushed is not live.
    OnMain,
    /// Its work is live: a deploy equipe saw succeed carried it — or, for a
    /// project that doesn't deploy, it is on the main branch.
    Live,
    /// Done (work that isn't code).
    Done,
    /// Every ticket of it was dropped.
    Cancelled,
}

impl Phase {
    /// Nothing more happens to it.
    pub fn is_finished(self) -> bool {
        matches!(self, Phase::Live | Phase::Done | Phase::Cancelled)
    }
}

/// A part of a request: its first ticket, how far it got, who is on it, since when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub ticket: TicketId,
    pub title: String,
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// When it got where it is (unix seconds), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// On main and not live, why, as a code (`deploy_failed`,
    /// `deploy_blocked`, `deploy_invalid`, `deploy_waits`); `None` while a
    /// deploy is simply on its way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_live: Option<String>,
}

/// A ticket of the request that waits on the person: put down for them, or
/// asking them a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wait {
    pub ticket: TicketId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
}

/// A ticket in the request's timeline, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub ticket: TicketId,
    pub title: String,
    pub status: TicketStatus,
    pub link: Link,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// When it last changed (unix seconds), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
    /// Its work is on the main branch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merged: bool,
}

/// Where a request stands, worked out from its tickets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waits: Vec<Wait>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Every part finished, nothing waits on the person, and a work ticket
    /// serves it — equipe's own checks never cover a request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Done, and none of its work merged or deployed anything: a decision or
    /// an instruction answered ("go with B"), with nothing to put live —
    /// never counted as waiting for production.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub decided: bool,
    /// No work ticket serves it a few minutes after it was asked: it is put
    /// to the PM, and stays open, until one does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uncovered: bool,
    /// Closed by a member with its work not done: why, as the person is told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_why: Option<String>,
}

/// Something the person asked for, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    /// In the person's words: the first line of what they said, cut short.
    pub title: String,
    /// The conversation they asked in, where its news goes. `None`: they put
    /// the ticket on the board themselves, with no conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    /// The member they asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// When they asked (unix seconds).
    pub asked_at: u64,
    pub tickets: Vec<RequestTicket>,
    pub status: Status,
    /// When it was all done and the person told: a screen lists it a while
    /// longer, then not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
}

/// What a request's news is about, the most telling first: one line says it
/// all, however many tickets moved since the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Milestone {
    /// Everything asked for is finished.
    AllDone,
    /// Something of it now waits on the person.
    NeedsYou,
    /// A run of it stopped; only the person brings it back.
    Blocked,
    /// Work of it is live.
    Live,
    /// Work of it reached the main branch, not live yet.
    OnMain,
    /// Its deploy failed or couldn't run: on main, not live, and why.
    NotLive,
    /// A ticket of it was done (a spec, a check, accepted work).
    Done,
    /// Work of it was handed in for review.
    HandedIn,
}

impl Milestone {
    pub fn code(self) -> &'static str {
        match self {
            Milestone::AllDone => "all_done",
            Milestone::NeedsYou => "needs_you",
            Milestone::Blocked => "blocked",
            Milestone::Live => "live",
            Milestone::OnMain => "on_main",
            Milestone::NotLive => "not_live",
            Milestone::Done => "done",
            Milestone::HandedIn => "handed_in",
        }
    }
}

pub type StopId = u64;

/// A kind of work the person stopped: its open tickets were cancelled, and
/// no member puts a new ticket of it on the board until the person lifts the
/// stop. A ticket is of it when it comes of one of its `tickets` (made while
/// working one, a follow-up of one), serves its `request`, or its title has
/// one of its `terms` (each word of a term) or names one of its `paths`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stop {
    pub id: StopId,
    /// What it is, in the person's words.
    pub words: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestId>,
    /// The chain of tickets it was stopped on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
    /// `you`, or the member the person told to stop it.
    pub by: String,
    /// Since when (unix seconds).
    pub since: u64,
    /// The open tickets it cancelled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled: Vec<TicketId>,
    /// New tickets of it turned away since.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub refused: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Work the PM decided for the person without a card — small, low-risk —
/// for their daily digest ("Decided for you", under Your requests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decided {
    pub ticket: TicketId,
    pub title: String,
    /// What changes for people, in a plain line (empty: it doesn't say).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub what: String,
    /// Accepted; else sent back.
    pub accepted: bool,
    /// When (unix seconds).
    pub at: u64,
    /// Landed by its own member, its gates passed — not by the PM's
    /// acceptance: the member's name. Revert or follow up from the digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landed_by: Option<String>,
}
