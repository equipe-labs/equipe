//! How much of a plan a request uses: its *load*, each kind of token weighed
//! as the model's published API prices weigh it. A subscription plan isn't
//! billed by these prices, but its limits track the cost of what is asked,
//! and the ratios are what matter: a cache read is about a tenth of fresh
//! input, a cache write a quarter more than it (twice it, kept for an hour),
//! output several times input. Raw token totals are dominated by cache reads
//! and say little about what drained a plan.
//!
//! One table, here, kept by hand from the providers' published price pages —
//! never fetched. A model not in it is weighed as its family's newest. Load
//! is in millionths of a dollar at those prices, but never shown as money:
//! only shares of it are.

use crate::protocol::Engine;
use crate::protocol::plan_use::Tokens;

/// A model family's prices, per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prices {
    pub input: f64,
    pub output: f64,
    /// Written to the cache for five minutes.
    pub cache_write: f64,
    /// Written to the cache for an hour.
    pub cache_write_long: f64,
    pub cache_read: f64,
}

/// A family whose cache writes cost 1.25× input (2× for an hour) and reads
/// `read` of input — Anthropic's published ratios.
const fn claude(input: f64, output: f64, read: f64) -> Prices {
    Prices { input, output, cache_write: input * 1.25, cache_write_long: input * 2.0, cache_read: input * read }
}

/// A family whose cache writes cost what input does and reads `read` of it —
/// OpenAI's.
const fn openai(input: f64, output: f64, read: f64) -> Prices {
    Prices { input, output, cache_write: input, cache_write_long: input, cache_read: input * read }
}

/// Claude's families, the first whose name is in the model's id: the more
/// particular first. Prices as published (USD per million tokens).
const CLAUDE: &[(&str, Prices)] = &[
    ("fable-5-1", claude(10.0, 50.0, 0.025)),
    ("mythos-5-1", claude(10.0, 50.0, 0.025)),
    ("fable", claude(10.0, 50.0, 0.1)),
    ("mythos", claude(10.0, 50.0, 0.1)),
    ("opus-5-5", claude(4.0, 20.0, 0.05)),
    ("opus-5", claude(5.0, 25.0, 0.1)),
    ("opus-4-8", claude(5.0, 25.0, 0.1)),
    ("opus-4-7", claude(5.0, 25.0, 0.1)),
    ("opus-4-6", claude(5.0, 25.0, 0.1)),
    ("opus-4-5", claude(5.0, 25.0, 0.1)),
    // Opus 4.1 and before.
    ("opus-4", claude(15.0, 75.0, 0.1)),
    ("opus-3", claude(15.0, 75.0, 0.1)),
    ("sonnet-5", claude(2.0, 10.0, 0.1)),
    ("sonnet", claude(3.0, 15.0, 0.1)),
    ("haiku-5", claude(0.1, 0.5, 0.1)),
    ("haiku-4", claude(1.0, 5.0, 0.1)),
    ("haiku", claude(0.8, 4.0, 0.1)),
    // Claude Code's aliases, as they resolve today.
    ("opus", claude(4.0, 20.0, 0.05)),
];

/// A Claude model not in the table: its newest Opus.
const CLAUDE_ELSE: Prices = claude(4.0, 20.0, 0.05);

/// OpenAI's (Codex's) families, the same way.
const OPENAI: &[(&str, Prices)] = &[("nano", openai(0.05, 0.4, 0.1)), ("mini", openai(0.25, 2.0, 0.1))];

/// A Codex model not in the table: GPT-5's prices.
const OPENAI_ELSE: Prices = openai(1.25, 10.0, 0.1);

/// The prices `model` is weighed at on `engine`.
pub fn prices(engine: Engine, model: &str) -> Prices {
    let model = model.to_ascii_lowercase().replace('.', "-");
    let (table, fallback) = match engine {
        Engine::Claude => (CLAUDE, CLAUDE_ELSE),
        Engine::Codex => (OPENAI, OPENAI_ELSE),
    };
    table.iter().find(|(name, _)| model.contains(name)).map_or(fallback, |(_, p)| *p)
}

/// The load of `tokens` on `model`, `long` of its cache writes kept an hour:
/// millionths of a dollar at the model's API prices.
pub fn load(engine: Engine, model: &str, tokens: Tokens, long: u64) -> u64 {
    let p = prices(engine, model);
    let long = long.min(tokens.cache_write);
    let micro = tokens.input as f64 * p.input
        + tokens.output as f64 * p.output
        + (tokens.cache_write - long) as f64 * p.cache_write
        + long as f64 * p.cache_write_long
        + tokens.cache_read as f64 * p.cache_read;
    micro.round() as u64
}

/// The load of each kind alone: input, output, cache writes, cache reads.
pub fn parts(engine: Engine, model: &str, tokens: Tokens, long: u64) -> [u64; 4] {
    let only = |t: Tokens, long: u64| load(engine, model, t, long);
    [
        only(Tokens { input: tokens.input, ..Default::default() }, 0),
        only(Tokens { output: tokens.output, ..Default::default() }, 0),
        only(Tokens { cache_write: tokens.cache_write, ..Default::default() }, long),
        only(Tokens { cache_read: tokens.cache_read, ..Default::default() }, 0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(input: u64, output: u64, cache_write: u64, cache_read: u64) -> Tokens {
        Tokens { input, output, cache_write, cache_read }
    }

    #[test]
    fn a_cache_read_weighs_a_fraction_of_input_and_a_write_a_quarter_more() {
        let read = load(Engine::Claude, "claude-sonnet-4-6", t(0, 0, 0, 1_000_000), 0);
        let input = load(Engine::Claude, "claude-sonnet-4-6", t(1_000_000, 0, 0, 0), 0);
        let write = load(Engine::Claude, "claude-sonnet-4-6", t(0, 0, 1_000_000, 0), 0);
        let hour = load(Engine::Claude, "claude-sonnet-4-6", t(0, 0, 1_000_000, 0), 1_000_000);
        let output = load(Engine::Claude, "claude-sonnet-4-6", t(0, 1_000_000, 0, 0), 0);
        assert_eq!((input, output), (3_000_000, 15_000_000), "Sonnet 4.6: $3 / $15 a million");
        assert_eq!(read * 10, input);
        assert_eq!(write * 4, input * 5);
        assert_eq!(hour, input * 2, "an hour's write: twice input");
    }

    #[test]
    fn each_family_is_weighed_by_its_own_prices_the_particular_first() {
        let input = |m: &str| prices(Engine::Claude, m).input;
        assert_eq!(input("claude-opus-5-5"), 4.0);
        assert_eq!(input("claude-opus-5"), 5.0, "Opus 5 isn't Opus 5.5");
        assert_eq!(input("claude-opus-4-1-20250805"), 15.0);
        assert_eq!(input("claude-sonnet-5-5"), 2.0);
        assert_eq!(input("claude-haiku-4-5-20251001"), 1.0);
        assert_eq!(input("opus"), 4.0, "an alias");
        assert_eq!(input("claude-something-new"), 4.0, "unknown: the newest Opus");
        assert_eq!(prices(Engine::Claude, "claude-opus-5-5").cache_read, 0.2, "Opus 5.5 reads at $0.20");
        assert_eq!(prices(Engine::Codex, "gpt-5.6-sol").input, 1.25);
        assert_eq!(prices(Engine::Codex, "gpt-5-mini").input, 0.25);
        assert_eq!(prices(Engine::Codex, "gpt-5.6-sol").cache_write, 1.25, "OpenAI charges no more to cache");
    }

    #[test]
    fn a_long_conversation_is_mostly_tokens_but_not_mostly_load() {
        // 98% of the tokens are cache reads, as a long session's are.
        let tokens = t(10_000, 150_000, 1_840_000, 98_000_000);
        let [input, output, write, read] = parts(Engine::Claude, "claude-opus-5-5", tokens, 0);
        let all = load(Engine::Claude, "claude-opus-5-5", tokens, 0);
        assert_eq!(input + output + write + read, all);
        assert!(read * 100 / all < 70, "cache reads: 98% of tokens, well under that of load ({read} of {all})");
        assert!(output > input * 10);
    }
}
