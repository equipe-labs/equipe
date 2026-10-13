//! The outcome of one model turn, whichever engine ran it.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Input read from the prompt cache or written to it: most of what a
    /// long conversation sends.
    pub cache_tokens: Option<u64>,
    /// How full the conversation's context was at the turn's last request.
    pub context_tokens: Option<u64>,
    /// The model's context window.
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResult {
    /// The assistant's text for this turn.
    pub text: String,
    /// Terminal status reported by the engine, e.g. `completed` or `incomplete`.
    pub status: String,
    pub usage: Option<TurnUsage>,
}
