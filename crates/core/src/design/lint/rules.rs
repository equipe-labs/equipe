//! The rules `equipe design lint` holds a screen to, as data: each has an id
//! (what a waiver and a finding name it by), a tier, how it is checked, what
//! it looks for and how to put it right.
//!
//! - **Floor**: never waived. A screen that breaks one isn't handed in.
//! - **Habit**: what generated screens fall into by default. Fixed, or kept
//!   with a reason written in the design's decision log.
//! - **Finish**: craft. Said, never blocking.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Floor,
    Habit,
    Finish,
}

impl Tier {
    pub fn word(self) -> &'static str {
        match self {
            Tier::Floor => "floor",
            Tier::Habit => "habit",
            Tier::Finish => "finish",
        }
    }
}

/// Where a rule is checked: in the source as written, in the page as a
/// browser draws it at each width, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Source,
    Render,
    Both,
}

impl How {
    pub fn in_source(self) -> bool {
        matches!(self, How::Source | How::Both)
    }

    pub fn in_render(self) -> bool {
        matches!(self, How::Render | How::Both)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub tier: Tier,
    pub how: How,
    /// What it looks for, in a line.
    pub checks: &'static str,
    /// How to put it right.
    pub fix: &'static str,
}

pub const RULES: &[Rule] = &[
    // ── floor ──
    Rule {
        id: "token-colour",
        tier: Tier::Floor,
        how: How::Source,
        checks: "Every colour comes from the design system: no hex, rgb() or named colour, and no utility palette the tokens don't have.",
        fix: "Use the role token (`var(--color-text)`, the system's class); if no token fits, ask for one rather than invent a value.",
    },
    Rule {
        id: "token-type",
        tier: Tier::Floor,
        how: How::Source,
        checks: "Type comes from the system: no family the tokens don't name, no font size off its type scale.",
        fix: "Use the type style or family token; a size between two steps takes the nearer step.",
    },
    Rule {
        id: "token-space",
        tier: Tier::Floor,
        how: How::Source,
        checks: "Margins, padding and gaps sit on the spacing scale.",
        fix: "Use the spacing token; a value between two steps takes the nearer one.",
    },
    Rule {
        id: "focus-visible",
        tier: Tier::Floor,
        how: How::Both,
        checks: "Every control shows where the keyboard is: no outline taken away without a focus style in its place.",
        fix: "Give `:focus-visible` the system's focus ring (outline or ring token, 3:1 against what it sits on).",
    },
    Rule {
        id: "reduced-motion",
        tier: Tier::Floor,
        how: How::Source,
        checks: "Whatever moves has a still version for people who ask for less motion.",
        fix: "Add `@media (prefers-reduced-motion: reduce)` (or `motion-reduce:`, `useReducedMotion`) that stops or shortens it.",
    },
    Rule {
        id: "invented-data",
        tier: Tier::Floor,
        how: How::Source,
        checks: "No invented proof: user counts, uptime, ratings, testimonials, customer logos, placeholder names or lorem ipsum.",
        fix: "Use the product's real words and figures (mark the element `data-source=\"…\"` with where they come from), or leave the place out.",
    },
    Rule {
        id: "control-name",
        tier: Tier::Floor,
        how: How::Source,
        checks: "Every button, link and field has a name a screen reader can say; every picture an alt.",
        fix: "Give it visible text, a `<label>`, or `aria-label`; a decorative picture `alt=\"\"`.",
    },
    Rule {
        id: "no-overflow",
        tier: Tier::Floor,
        how: How::Render,
        checks: "Nothing runs off the side at any width checked: the page never scrolls sideways.",
        fix: "Let the wide element wrap, shrink (`min-width: 0`, `max-width: 100%`) or scroll inside itself.",
    },
    Rule {
        id: "contrast",
        tier: Tier::Floor,
        how: How::Render,
        checks: "Text reaches 4.5:1 on what it sits on (3:1 when large), at every width.",
        fix: "Use the text role meant for that surface; never a lighter grey for looks.",
    },
    Rule {
        id: "text-clipped",
        tier: Tier::Floor,
        how: How::Render,
        checks: "No text is cut off by its box at any width.",
        fix: "Let the box grow with its text, or wrap it; truncate only with an ellipsis and the whole text reachable.",
    },
    Rule {
        id: "primary-in-view",
        tier: Tier::Floor,
        how: How::Render,
        checks: "The screen's primary action (marked `data-primary`) is in the first screenful at every width.",
        fix: "Bring it up, or keep it in view (a bar that stays); move what pushes it down.",
    },
    // ── habits ──
    Rule {
        id: "eyebrow",
        tier: Tier::Habit,
        how: How::Source,
        checks: "A small label over every heading.",
        fix: "Let the heading say it; keep a label only where it tells something the heading can't.",
    },
    Rule {
        id: "tracked-caps",
        tier: Tier::Habit,
        how: How::Source,
        checks: "Small capitals with wide letter spacing as labels.",
        fix: "Sentence case in the label style; capitals only where the brand sets them.",
    },
    Rule {
        id: "coloured-word",
        tier: Tier::Habit,
        how: How::Source,
        checks: "One word of a headline in another colour or a gradient.",
        fix: "One colour for the headline; let the words carry the weight.",
    },
    Rule {
        id: "gradient-hero",
        tier: Tier::Habit,
        how: How::Source,
        checks: "A gradient behind the first screen.",
        fix: "The system's surface; colour where it means something.",
    },
    Rule {
        id: "nested-cards",
        tier: Tier::Habit,
        how: How::Source,
        checks: "A card inside a card.",
        fix: "One surface; group inside it with space or a divider.",
    },
    Rule {
        id: "emoji-icons",
        tier: Tier::Habit,
        how: How::Source,
        checks: "Emoji standing in for icons.",
        fix: "The system's icon set, at its sizes, in the text colour.",
    },
    Rule {
        id: "all-centred",
        tier: Tier::Habit,
        how: How::Both,
        checks: "Nearly everything centred.",
        fix: "Align to a start edge; centre only what stands alone.",
    },
    Rule {
        id: "uniform-blocks",
        tier: Tier::Habit,
        how: How::Both,
        checks: "The same radius and shadow on every block.",
        fix: "Vary by role: what lifts has a shadow, what sits flat has none.",
    },
    Rule {
        id: "decorative-dots",
        tier: Tier::Habit,
        how: How::Source,
        checks: "Pulsing or decorative dots that report no real status.",
        fix: "A dot only beside a status it reports, with its words.",
    },
    Rule {
        id: "pretend-submit",
        tier: Tier::Habit,
        how: How::Source,
        checks: "A form that says it worked without sending anything.",
        fix: "Wire it to what it does, or show it as a prototype with its states named.",
    },
    // ── finish ──
    Rule {
        id: "literal-on-scale",
        tier: Tier::Finish,
        how: How::Source,
        checks: "A value written out that a token already holds.",
        fix: "Name the token, so a change to the system reaches it.",
    },
    Rule {
        id: "tabular-figures",
        tier: Tier::Finish,
        how: How::Source,
        checks: "Figures in a table or column without tabular figures.",
        fix: "`font-variant-numeric: tabular-nums` where numbers line up.",
    },
    Rule {
        id: "heading-order",
        tier: Tier::Finish,
        how: How::Source,
        checks: "Heading levels that skip one.",
        fix: "Go down one level at a time; style is the type token's, not the level's.",
    },
    Rule {
        id: "image-size",
        tier: Tier::Finish,
        how: How::Source,
        checks: "Pictures without a width and height, so the page jumps as they load.",
        fix: "Give `width` and `height` (or an aspect ratio).",
    },
    Rule {
        id: "line-length",
        tier: Tier::Finish,
        how: How::Render,
        checks: "Lines of text much longer than reads well.",
        fix: "Keep body text to about 45–80 characters a line.",
    },
    Rule {
        id: "touch-target",
        tier: Tier::Finish,
        how: How::Render,
        checks: "Controls smaller than a finger at phone widths.",
        fix: "44×44 at least, with the hit area grown if the drawing stays small.",
    },
];

/// The rule called `id`.
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rule_has_a_unique_id_a_tier_what_it_checks_and_a_fix() {
        let mut seen = std::collections::HashSet::new();
        for r in RULES {
            assert!(seen.insert(r.id), "{} twice", r.id);
            assert!(r.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{}", r.id);
            assert!(r.checks.ends_with('.') && r.fix.ends_with('.'), "{}", r.id);
        }
        for tier in [Tier::Floor, Tier::Habit, Tier::Finish] {
            assert!(RULES.iter().filter(|r| r.tier == tier).count() >= 6, "{tier:?}");
        }
        assert_eq!(rule("contrast").map(|r| r.tier), Some(Tier::Floor));
        assert!(rule("nope").is_none());
    }
}
