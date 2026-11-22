//! What keeps a hosted team's model spend honest: who spent it, and a few
//! limits on what a turn drags along.
//!
//! These apply on the **credits** profile only (`EQUIPE_AI=credits`, set by
//! the hosted team's machine setup): there every model call goes through the
//! platform's gateway and is billed per call, so the platform's ledger can
//! say where the money went. A person's own Claude plan, or any other
//! provider, never gets any of it.
//!
//! - **Attribution** ([`headers`]): each Claude Code turn carries who it is
//!   for (member, role), which ticket, and what kind of turn it is
//!   ([`TurnKind`]) as `x-equipe-*` request headers, through Claude Code's
//!   own `ANTHROPIC_CUSTOM_HEADERS`. The gateway strips them before the
//!   request goes upstream and keeps them in the ledger line. Values are
//!   short names and numbers — never a prompt, a path or a person's words.
//! - **Output caps** ([`output_caps`]): Claude Code's own Bash and MCP answers
//!   go into the conversation and are re-read on every later call, like
//!   equipe's own tools' answers (`tool_cap`); its defaults are twice as
//!   long as those. Reading a file is left alone: a cut file read costs more
//!   steps than it saves.
//! - **Explicit effort** ([`lane_effort`]): a lane whose ticket kind has no
//!   tier effort doesn't fall back to Claude Code's `xhigh`.

use crate::roles;

/// The environment variable that turns the profile on.
pub const PROFILE_ENV: &str = "EQUIPE_AI";
/// Its value on a hosted team whose model calls are paid in equipe credits.
pub const CREDITS: &str = "credits";

/// Whether this daemon runs a hosted team on equipe credits.
pub fn credits_profile() -> bool {
    is_credits(std::env::var(PROFILE_ENV).ok().as_deref())
}

fn is_credits(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case(CREDITS))
}

/// What a turn is for, as the ledger tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    /// The PM, woken by equipe (a notice, a hand-in, a routine) — nobody spoke.
    PmWake,
    /// The PM, answering the person or planning.
    Pm,
    /// A member's lane on a ticket.
    Ticket,
    /// A designer's lane on a design ticket.
    Design,
    /// A design QA session (the light reviewer).
    DesignQa,
    /// A member's conversation at its desk, off the board.
    Desk,
}

impl TurnKind {
    pub fn label(self) -> &'static str {
        match self {
            TurnKind::PmWake => "pm_wake",
            TurnKind::Pm => "pm",
            TurnKind::Ticket => "ticket",
            TurnKind::Design => "design",
            TurnKind::DesignQa => "design_qa",
            TurnKind::Desk => "desk",
        }
    }

    /// The kind of a turn by who runs it and where.
    pub fn of(role: &str, on_ticket: bool, light_review: bool, woken: bool) -> TurnKind {
        if role == roles::PM {
            return if woken && !on_ticket { TurnKind::PmWake } else { TurnKind::Pm };
        }
        match (on_ticket, light_review, role == "designer") {
            (true, true, _) => TurnKind::DesignQa,
            (true, false, true) => TurnKind::Design,
            (true, false, false) => TurnKind::Ticket,
            (false, ..) => TurnKind::Desk,
        }
    }
}

/// A header value kept to a short, plain token: letters, digits and `-_.@`.
fn plain(value: &str) -> String {
    let kept: String = value.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@')).take(48).collect();
    if kept.is_empty() { "x".to_owned() } else { kept }
}

/// The `ANTHROPIC_CUSTOM_HEADERS` text for a turn: one `Name: Value` line per header.
pub fn headers(member: &str, role: &str, ticket: Option<u64>, kind: TurnKind) -> String {
    let mut lines = vec![format!("x-equipe-member: {}", plain(member)), format!("x-equipe-role: {}", plain(role))];
    if let Some(ticket) = ticket {
        lines.push(format!("x-equipe-ticket: T-{ticket}"));
    }
    lines.push(format!("x-equipe-turn: {}", kind.label()));
    lines.join("\n")
}

/// The Claude Code limits on its own tools' answers, as environment pairs.
/// Bash output in characters (Claude Code's default is 30000), MCP answers
/// in tokens (its default is 25000; equipe's own tools are cut at 10000).
pub fn output_caps() -> [(String, String); 2] {
    [("BASH_MAX_OUTPUT_LENGTH".to_owned(), "15000".to_owned()), ("MAX_MCP_OUTPUT_TOKENS".to_owned(), crate::tool_cap::MAX_TOOL_TOKENS.to_string())]
}

/// The effort of a lane that no tier or person gave one: medium for the
/// roles that build and check, where the work is routine once it is specified
/// (the ticket's tier decides whenever there is one). `None`: the engine's
/// own default stays — the PM's and the designer's thinking is the work.
pub fn lane_effort(role: &str, kind: TurnKind) -> Option<&'static str> {
    match (kind, role) {
        (TurnKind::Ticket, r) if r != roles::PM && r != "designer" => Some("medium"),
        _ => None,
    }
}

/// `existing` custom headers with `added` after them: nothing the person or
/// the machine already set is dropped.
pub fn merged(existing: Option<&str>, added: &str) -> String {
    match existing.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => format!("{e}\n{added}"),
        None => added.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_credits_only() {
        assert!(is_credits(Some("credits")));
        assert!(is_credits(Some(" Credits ")));
        assert!(!is_credits(Some("own")));
        assert!(!is_credits(None));
    }

    #[test]
    fn turn_kinds_by_who_and_where() {
        assert_eq!(TurnKind::of("pm", false, false, true), TurnKind::PmWake);
        assert_eq!(TurnKind::of("pm", false, false, false), TurnKind::Pm);
        assert_eq!(TurnKind::of("frontend", true, false, false), TurnKind::Ticket);
        assert_eq!(TurnKind::of("designer", true, false, false), TurnKind::Design);
        assert_eq!(TurnKind::of("designer", true, true, false), TurnKind::DesignQa);
        assert_eq!(TurnKind::of("backend", false, false, false), TurnKind::Desk);
    }

    #[test]
    fn headers_are_plain_tokens() {
        let h = headers("front end\nx-evil: 1", "frontend", Some(7), TurnKind::Ticket);
        assert_eq!(h, "x-equipe-member: frontendx-evil1\nx-equipe-role: frontend\nx-equipe-ticket: T-7\nx-equipe-turn: ticket");
        assert_eq!(h.lines().count(), 4, "no value can add a header");
        let desk = headers("pm", "pm", None, TurnKind::PmWake);
        assert!(!desk.contains("ticket"));
        assert!(desk.ends_with("x-equipe-turn: pm_wake"));
        assert!(plain("").starts_with('x'));
    }

    #[test]
    fn headers_keep_what_was_already_set() {
        assert_eq!(merged(None, "a: 1"), "a: 1");
        assert_eq!(merged(Some("  "), "a: 1"), "a: 1");
        assert_eq!(merged(Some("b: 2"), "a: 1"), "b: 2\na: 1");
    }

    #[test]
    fn only_building_lanes_get_an_explicit_effort() {
        assert_eq!(lane_effort("frontend", TurnKind::Ticket), Some("medium"));
        assert_eq!(lane_effort("qa", TurnKind::Ticket), Some("medium"));
        assert_eq!(lane_effort("designer", TurnKind::Design), None);
        assert_eq!(lane_effort("designer", TurnKind::Ticket), None);
        assert_eq!(lane_effort("pm", TurnKind::Pm), None);
        assert_eq!(lane_effort("pm", TurnKind::PmWake), None);
    }

    #[test]
    fn output_caps_are_below_claude_codes_defaults() {
        let caps = output_caps();
        assert!(caps[0].1.parse::<u32>().unwrap() < 30_000);
        assert!(caps[1].1.parse::<u32>().unwrap() < 25_000);
    }
}
