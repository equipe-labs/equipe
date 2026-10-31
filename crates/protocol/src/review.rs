//! A second look at a member's work by an NVIDIA Nemotron model on Nebius
//! Token Factory: when a ticket is handed in, the model reads the branch's
//! diff against the ticket and says what the person should look at before
//! accepting. Advice only — it never accepts, merges or sends work back.

use serde::{Deserialize, Serialize};

use crate::TicketId;

/// How the work reads to the reviewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    /// Nothing stood out.
    LooksGood,
    /// Worth a closer look before accepting.
    #[default]
    NeedsAttention,
    /// Likely wrong, unsafe or incomplete.
    Risky,
}

impl ReviewVerdict {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "looks_good" | "good" | "ok" | "approve" | "approved" | "pass" => Some(Self::LooksGood),
            "needs_attention" | "attention" | "comment" | "minor" => Some(Self::NeedsAttention),
            "risky" | "risk" | "reject" | "request_changes" | "changes" | "fail" => Some(Self::Risky),
            _ => None,
        }
    }
}

/// One thing in the diff to look at: where, and why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewPoint {
    /// The file it is in, as the diff names it; empty when it is about the whole.
    #[serde(default)]
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub why: String,
}

/// A ticket's review, as the model gave it (cleaned: plain text, bounded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeReview {
    pub ticket: TicketId,
    /// The model that wrote it, as Token Factory names it (`nvidia/…`).
    pub model: String,
    pub verdict: ReviewVerdict,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<ReviewPoint>,
    /// What the ticket asks for, or the change risks, that no test covers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_tests: Vec<String>,
    /// The diff was longer than the reviewer reads: only its start was read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Unix seconds.
    #[serde(default)]
    pub at: u64,
}
