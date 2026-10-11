//! The outcome of one model turn, whichever engine ran it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResult {
    /// The assistant's text for this turn.
    pub text: String,
    /// Terminal status reported by the engine, e.g. `completed` or `incomplete`.
    pub status: String,
    pub usage: Option<TurnUsage>,
}
