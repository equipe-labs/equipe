//! The design setup's two tiers, and the hard budget each step runs under.
//! No runtime types here: the ticket's words in, the tier, step and budgets out.
//!
//! A new product's design setup used to be four Opus-heavy tickets (the brief, a logo kit, tokens with component
//! sheets and guidelines, a UI map): about $47 of a paying team's credits on one product, most of it before the person
//! had seen a screen. Now it has two tiers (the designer's `design-system` skill says what each holds):
//!
//! - **Starter brand** ([`DesignTier::Starter`]), every new product's default: the platform's deterministic tools do
//!   the work (palette from the direction or the existing site, a type pairing, the gated logo pipeline, one photograph,
//!   `tokens.json`, a brand sheet with a homepage hero drawn from the product's own components); the designer takes one
//!   art-direction turn that makes the words and the pick specific to the business. A blind review put the tools with
//!   one short standard-tier look at 6/10 for first impression against the full setup's 8 (2026-11-26), so the turn
//!   runs on the strong tier (`crate::tiers`), still under a small hard budget ([`STARTER_TOKENS`], [`STARTER_TURN`]).
//! - **Deep brand system** ([`DesignTier::Deep`]), only when the person asks for it (a paid upgrade they choose on the
//!   Brand page, with its cost shown) or accepts the PM's one suggestion: the full brief, component sheets, guidelines
//!   and UI map, a ticket a step, each under its own hard budget ([`DeepStep::tokens`], [`DeepStep::turn`]).
//!
//! A step past its budget stops early: mid-turn the member is told once to finish with what it has
//! (`runtime::design_qa`), and once its turns have spent the step's tokens the ticket goes to the PM like any ticket
//! over budget (`crate::spend`). The budgets are hard: they hold even where the squad set no ticket budget of its own.

/// Which tier a design setup ticket is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DesignTier {
    Starter,
    Deep,
}

/// One step of the deep brand system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeepStep {
    /// The full brief: audience, attributes, references, voice.
    Brief,
    /// A logo round: the deep system's own, or any designer ticket that is a logo ([`logo_work`]). The platform's
    /// `logo_round` tool does the round (brief, six candidates, the gate, one critique, a decision card), so the
    /// designer only drives it: the standard tier (`crate::tiers`) under a small budget.
    Logo,
    /// Layered tokens, the component sheets and the guidelines.
    System,
    /// The UI map: every page of the product pictured in its states.
    Map,
}

/// A design setup ticket's tier and, for the deep tier, its step (`None`: the deep system as one ticket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Step {
    pub tier: DesignTier,
    pub step: Option<DeepStep>,
}

/// Tokens the starter's art-direction turn may use: about $1.05 of strong-tier turns at most, mostly cache reads (a
/// measured turn: 10–12 tool calls, under 100k tokens of context), so with the logo round and the photograph the whole
/// starter brand stays near $2 of credits even at its cap.
pub const STARTER_TOKENS: u64 = 1_500_000;
/// Tokens a logo round may use. A logo ticket once used 19.4M tokens (five picture rounds, nine Opus critiques, about
/// $20); with `logo_round` a round is one tool call, the decision card and a hand-in, and a pick or a refinement one more.
pub const LOGO_TOKENS: u64 = 1_500_000;
/// The starter's art-direction turn: tool calls, seconds.
pub const STARTER_TURN: (u32, u64) = (30, 15 * 60);
/// A deep step's turn when it names no step.
const DEEP_TURN: (u32, u64) = (140, 50 * 60);

impl DeepStep {
    /// Tokens one deep step may use before it stops. Their sum is what the Brand page's estimate is drawn from.
    pub fn tokens(self) -> u64 {
        match self {
            DeepStep::Brief => 3_000_000,
            DeepStep::Logo => LOGO_TOKENS,
            DeepStep::System => 8_000_000,
            DeepStep::Map => 6_000_000,
        }
    }

    /// One turn's cap on the step: tool calls, seconds.
    pub fn turn(self) -> (u32, u64) {
        match self {
            DeepStep::Brief => (60, 20 * 60),
            DeepStep::Logo => (40, 15 * 60),
            DeepStep::System => (140, 50 * 60),
            DeepStep::Map => (120, 45 * 60),
        }
    }
}

/// Every deep step's tokens together: the most the deep system can spend.
pub fn deep_tokens() -> u64 {
    [DeepStep::Brief, DeepStep::Logo, DeepStep::System, DeepStep::Map].iter().map(|s| s.tokens()).sum()
}

impl Step {
    /// The tokens this step may use.
    pub fn tokens(self) -> u64 {
        match (self.tier, self.step) {
            (DesignTier::Starter, _) => STARTER_TOKENS,
            (DesignTier::Deep, Some(step)) => step.tokens(),
            (DesignTier::Deep, None) => deep_tokens(),
        }
    }

    /// One turn's cap: tool calls, seconds.
    pub fn turn(self) -> (u32, u64) {
        match (self.tier, self.step) {
            (DesignTier::Starter, _) => STARTER_TURN,
            (DesignTier::Deep, Some(step)) => step.turn(),
            (DesignTier::Deep, None) => DEEP_TURN,
        }
    }
}

/// The design setup step a ticket is, from its title and body, as the `working-with-design` skill has the PM write
/// it ("Starter brand", "Deep brand system: …"); `None` for any other ticket.
pub fn classify(title: &str, body: &str) -> Option<Step> {
    let title = title.to_lowercase();
    let body = body.to_lowercase();
    if title.contains("starter brand") || body.contains("design tier: starter") {
        return Some(Step { tier: DesignTier::Starter, step: None });
    }
    if !(title.contains("deep brand") || body.contains("design tier: deep")) {
        return logo_work(&title, &body).then_some(Step { tier: DesignTier::Deep, step: Some(DeepStep::Logo) });
    }
    let step = if title.contains("ui map") || title.contains("flows") {
        Some(DeepStep::Map)
    } else if title.contains("design system") || title.contains("tokens") || title.contains("component") || title.contains("guidelines") {
        Some(DeepStep::System)
    } else if title.contains("logo") {
        Some(DeepStep::Logo)
    } else if title.contains("brief") {
        Some(DeepStep::Brief)
    } else {
        None
    };
    Some(Step { tier: DesignTier::Deep, step })
}

/// A ticket whose work is making or refining a logo: the PM's `Design step: logo` line (titles are often in the
/// person's language), or a title naming the logo that is not about putting a finished one on the product.
pub fn logo_work(title: &str, body: &str) -> bool {
    let (title, body) = (title.to_lowercase(), body.to_lowercase());
    if body.lines().any(|l| l.trim().trim_start_matches(['-', '*', ' ']).starts_with("design step: logo")) {
        return true;
    }
    let names = title.split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "logo" | "logos" | "logomark" | "logotype"));
    let applies = ["put the", "apply the", "use the", "add the", "swap", "replace the", "on the site", "in the header", "favicon"].iter().any(|w| title.contains(w));
    let wider = ["design system", "brand system", "screen", "page", "redesign"].iter().any(|w| title.contains(w));
    names && !applies && !wider
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starter_is_read_from_its_title_or_its_tier_line() {
        assert_eq!(classify("Starter brand for Rep Club", ""), Some(Step { tier: DesignTier::Starter, step: None }));
        assert_eq!(classify("Brand pass", "Design tier: starter\nRun starter_brand."), Some(Step { tier: DesignTier::Starter, step: None }));
        assert_eq!(classify("Homepage hero", "Make the hero"), None);
        assert_eq!(classify("Starter kit for the API", ""), None, "the word starter alone is not the brand step");
    }

    #[test]
    fn a_deep_step_is_named_by_the_title() {
        let deep = |step| Some(Step { tier: DesignTier::Deep, step });
        assert_eq!(classify("Deep brand system: brief", ""), deep(Some(DeepStep::Brief)));
        assert_eq!(classify("Deep brand system: tokens, component sheets and guidelines", ""), deep(Some(DeepStep::System)));
        assert_eq!(classify("Deep brand system: UI map", ""), deep(Some(DeepStep::Map)));
        assert_eq!(classify("Deep brand system: logo round", ""), deep(Some(DeepStep::Logo)));
        assert_eq!(classify("Deep brand system", ""), deep(None));
        assert_eq!(classify("Brand system", "Design tier: deep"), deep(None));
    }

    #[test]
    fn a_logo_ticket_is_a_logo_round_by_its_title_or_its_design_step_line() {
        let logo = Some(Step { tier: DesignTier::Deep, step: Some(DeepStep::Logo) });
        assert_eq!(classify("Logo for OnePass", ""), logo);
        assert_eq!(classify("Refine the logo", "Make it calmer"), logo);
        assert_eq!(classify("Tighten the mark", "Design step: logo\nThe person wants it rounder."), logo, "a title in any language, the line in English");
        assert_eq!(classify("Put the new logo in the header", ""), None, "putting a finished logo on the site is frontend work");
        assert_eq!(classify("Replace the logo on the site", ""), None);
        assert_eq!(classify("Logo page redesign", ""), None);
        assert_eq!(classify("Catalog filters", ""), None, "a word that only contains the letters");
        assert_eq!(Step { tier: DesignTier::Deep, step: Some(DeepStep::Logo) }.tokens(), LOGO_TOKENS);
    }

    #[test]
    fn budgets_are_small_for_the_starter_and_capped_for_each_deep_step() {
        let starter = Step { tier: DesignTier::Starter, step: None };
        assert_eq!(starter.tokens(), STARTER_TOKENS);
        assert!(starter.tokens() < DeepStep::Brief.tokens());
        assert_eq!(deep_tokens(), 18_500_000);
        assert_eq!(Step { tier: DesignTier::Deep, step: None }.tokens(), deep_tokens());
        assert!(DeepStep::System.tokens() <= crate::spend::CREDITS_TICKET_BUDGET, "no deep step is allowed more than a credits ticket");
    }
}
