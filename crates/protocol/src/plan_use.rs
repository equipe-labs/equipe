//! Where a plan's use went: the share of an engine's week (or the last day)
//! each part of the machine took — the person's own sessions outside equipe,
//! each project, ticket and member — its burn hour by hour, and the spikes in
//! it, in plain words. A screen asks ([`crate::Command::PlanUse`] →
//! [`crate::Event::PlanUse`]); the daemon tells every screen when one ticket,
//! or the use outside equipe, burns a large share of the week in a short time
//! ([`crate::Event::PlanBurn`]).
//!
//! Shares are of the plan's *load*, not raw tokens: most tokens of a long
//! conversation are cheap cache reads, so each kind of token is weighed as the
//! model's published prices weigh it (`equipe_core::plan_use::weights`).
//! Percentages are in basis points (10 000 = 100 %).

use serde::{Deserialize, Serialize};

use crate::{Engine, PlanWindow, TicketId};

/// The stretch a look covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Span {
    /// The plan's current weekly window, from its start until now.
    #[default]
    Week,
    /// The last 24 hours.
    Day,
}

/// Tokens by kind, as the API counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Input neither read from nor written to the cache.
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    pub fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// Who used a share of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Who {
    /// The person's own Claude Code or Codex sessions, outside equipe; with a
    /// folder (`~/code/site`), those that ran there.
    Outside {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
    },
    /// A project of a squad, by its folder's name.
    Project { squad: Option<String>, project: String },
    /// A squad's own folder (its desk conversations, its tasks).
    Home { squad: String },
    Ticket { squad: Option<String>, ticket: TicketId },
    Member { squad: String, member: String },
    /// equipe's, but no squad's that it can tell.
    Equipe,
}

impl Who {
    pub fn is_outside(&self) -> bool {
        matches!(self, Who::Outside { .. })
    }
}

/// What a share went mostly on, as its load says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mostly {
    /// Reading a long conversation back from the cache, request after request.
    Rereading,
    /// Putting new context in the cache: big files, long tool output.
    Loading,
    /// Writing: answers, code, reasoning.
    Writing,
    /// Fresh input, not cached.
    Input,
}

/// One part of the plan's use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    pub who: Who,
    /// A ticket's title; a member's role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Its weighted load.
    pub load: u64,
    /// Of the load seen in the look's stretch.
    pub share_bp: u32,
    /// Of the plan's week, once the plan's use was read: its share of the
    /// week's load times how much of the week is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week_bp: Option<u32>,
    pub tokens: Tokens,
    pub calls: u64,
    pub mostly: Mostly,
    /// Outside equipe: by folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Slice>,
}

/// One bar of the burn: an hour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beat {
    /// Its start (Unix seconds).
    pub at: i64,
    pub load: u64,
    /// Of `load`, outside equipe's.
    pub outside: u64,
    pub calls: u64,
    /// What ran then, the most first (a few).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Slice>,
}

/// A stretch where one part burned fast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spike {
    pub from: i64,
    pub to: i64,
    pub slice: Slice,
}

/// How long the week lasts at the pace of late.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pace {
    /// When it runs out at this pace (Unix seconds).
    pub runs_out_at: i64,
    /// Whether that comes before the window starts over.
    pub before_reset: bool,
    /// Of the week, used per day at this pace.
    pub per_day_bp: u32,
}

/// Where an engine's plan went, over a stretch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub engine: Engine,
    pub span: Span,
    pub from: i64,
    pub to: i64,
    /// The plan's weekly window, as last read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week: Option<PlanWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<Pace>,
    pub load: u64,
    /// The load of the plan's whole week so far, which `week_bp`s are of.
    #[serde(default)]
    pub week_load: u64,
    pub tokens: Tokens,
    pub calls: u64,
    /// Outside equipe, all of it, with its folders as children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside: Option<Slice>,
    pub projects: Vec<Slice>,
    pub tickets: Vec<Slice>,
    pub members: Vec<Slice>,
    /// Hour by hour, oldest first.
    pub beats: Vec<Beat>,
    pub spikes: Vec<Spike>,
    /// The transcripts on the machine are still being read for the first
    /// time: what is shown so far is short of it.
    #[serde(default)]
    pub reading: bool,
}

/// One ticket, or the use outside equipe, burned a large share of the week
/// in a short time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Burn {
    /// Told once: the same burn keeps this.
    pub id: String,
    pub engine: Engine,
    pub who: Who,
    /// A ticket's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Of the week, in the stretch.
    pub week_bp: u32,
    /// The stretch, in hours, ending `at`.
    pub hours: u32,
    pub at: i64,
    pub calls: u64,
    pub mostly: Mostly,
}
