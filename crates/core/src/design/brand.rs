//! Brand work — a brand, a rebrand, a new visual direction — and only it
//! takes the designer's full reference pass and its tools (`refs`, `palette`,
//! `type`, `component`, the `design_tools` board tool). Every other ticket a
//! designer or an engineer takes is told exactly what it was told before.
//!
//! A ticket is brand work when its body has a line `Design: brand` (or
//! `Design: rebrand`, `Design: new direction`) — the PM's explicit flag — or
//! when its title or body asks for one in so many words: a rebrand, a brand
//! or visual identity, a new visual direction.

/// The guide put in a brand ticket's first prompt.
const GUIDE: &str = include_str!("../../assets/brand-pass.md");

/// Words that ask for brand work.
const ASKS: [&str; 8] = ["rebrand", "re-brand", "brand identity", "visual identity", "visual direction", "brand refresh", "new brand", "brand design"];

/// Whether a ticket is brand work: the PM's `Design: …` flag, or the ask in its words.
pub fn is_branding(title: &str, body: &str) -> bool {
    for line in body.lines() {
        if let Some(value) = line.trim().trim_start_matches(['-', '*', ' ']).strip_prefix("Design:").or_else(|| line.trim().strip_prefix("design:")) {
            let v = value.trim().to_ascii_lowercase();
            if v.starts_with("brand") || v.starts_with("rebrand") || v.starts_with("new direction") || v.starts_with("new visual direction") {
                return true;
            }
        }
    }
    let text = format!("{title}\n{body}").to_lowercase();
    ASKS.iter().any(|a| text.contains(a))
}

/// What a designer starting a brand ticket is told: the full pass, and how
/// it reaches its tools — the `design_tools` board tool with the board's
/// tools, `equipe design …` from a shell without them.
pub fn note(board_tools: bool) -> String {
    let tools = if board_tools {
        "Your tools for it run through the `design_tools` board tool, its `args` the words after `equipe design` \
(`[\"refs\", \"shoot\", \"<url>\", \"--learn\", \"…\", \"--aspect\", \"layout\"]`): equipe runs them for you outside your \
sandbox, in your worktree, reading only the listed reference sites — nothing to ask. One call at a time; several pages \
go in one `refs shoot`."
    } else {
        "Your tools for it are `equipe design refs | palette | type | component`; they read the web and draw in Chrome, so \
outside an engine's sandbox only."
    };
    format!("\n\n{}", GUIDE.replace("{{tools}}", tools).trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_work_is_known_by_the_pms_flag_or_by_its_own_words() {
        assert!(is_branding("Kettle", "Some context.\nDesign: brand\n"));
        assert!(is_branding("x", "- Design: new direction"));
        assert!(is_branding("Kettle's first visual direction", "Give it a look."));
        assert!(is_branding("Rebrand the site", ""));
        assert!(is_branding("Logo and colours", "We need a visual identity for the launch."));
        for build in [
            ("Design and build Quillo's landing page", "One static page from the brief; use the brand colours in the design system."),
            ("Design the Saved page's empty state", "Use the page's existing styles."),
            ("Fix the button contrast", "Design: QA pass on the brand page"),
        ] {
            assert!(!is_branding(build.0, build.1), "{build:?}");
        }
    }

    #[test]
    fn the_note_carries_the_full_pass_and_the_way_to_its_tools() {
        let with = note(true);
        assert!(with.contains("design_tools") && with.contains("refs board") && with.contains("ask_decision") && !with.contains("{{"), "{with}");
        assert!(with.contains("never another company's logo"), "the brand rule");
        let without = note(false);
        assert!(without.contains("equipe design refs") && !without.contains("`design_tools` board tool"));
    }
}
