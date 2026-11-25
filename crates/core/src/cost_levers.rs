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
//! - **Unused built-in tools left out** ([`UNUSED_TOOLS`]): every tool
//!   Claude Code offers is described in every call's prompt. A member's lane
//!   never used the scheduling, notebook, worktree, notification and remote
//!   tools (a paying team's 2,000 calls: zero), and leaving them out took the
//!   first call from 19.5k to 14.7k tokens; a blind A/B with the lighter
//!   prompt (2026-11-25, three tickets) found the same work.
//! - **Cache kept an hour where turns are sparse** ([`cache_ttl`]): the PM's
//!   and a member's desk conversation go on in the same engine session turn
//!   after turn, minutes apart. On an API key Claude Code caches the prompt
//!   for 5 minutes, so nearly every PM wake wrote its whole conversation
//!   (often 100k tokens) to the cache again. An hour's cache costs 1.6x per
//!   token written but is read back at the read price on the next wake: on a
//!   paying team's four days it would have cut the PM's cache writes from
//!   $8.07 to about $4. Lanes call every few seconds, so they keep 5 minutes
//!   (an hour would cost them more), and so do the PM's own subagents. A
//!   conversation that comes back after more than an hour unused has lost its
//!   cache either way and writes all of it again; it is the next wake that
//!   decides whether an hour's write pays, and after a long quiet the next
//!   wake seldom falls between five minutes and an hour (hosted teams' PM
//!   turns, 2026-11: about a third of the time; an hour pays from about two
//!   thirds), so that turn writes at the 5-minute price. An hour's request
//!   doesn't read what was written for 5 minutes (5 minutes' reads an hour's),
//!   so a wake within 5 minutes of such a turn keeps 5 minutes too, and the
//!   first wake after a longer gap goes back to the hour. Replayed on the
//!   hosted teams' PM calls this took their cache cost 11% below an hour
//!   throughout (one busy team: 10%; the busiest squad: 2%). The prompt itself
//!   is unchanged: this is the same work, cached for a different time.
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

/// The environment variable that names the ticket a member's run is on.
pub const TICKET_ENV: &str = "EQUIPE_TICKET";

/// [`TICKET_ENV`] for a run on `ticket`: `T-<n>`, the same token as the
/// `x-equipe-ticket` header, so tools the run starts can pass it on.
pub fn ticket_env(ticket: u64) -> (String, String) {
    (TICKET_ENV.to_owned(), format!("T-{ticket}"))
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

/// Claude Code's built-in tools a member's turn on credits never uses: left
/// out of the turn (`--disallowedTools`), so their descriptions are not resent
/// with every call. A name this Claude Code doesn't have is ignored.
pub const UNUSED_TOOLS: [&str; 13] = [
    "CronCreate", "CronDelete", "CronList", "DesignSync", "EnterWorktree", "ExitWorktree", "NotebookEdit", "PushNotification", "RemoteTrigger", "ReportFindings", "ScheduleWakeup", "Workflow", "McpTaskCheck",
];

/// How long Claude Code keeps a conversation's prompt cache when it is told
/// to keep it an hour.
pub const CACHE_HOUR: u64 = 60 * 60;
/// And when told 5 minutes.
pub const CACHE_SHORT: u64 = 5 * 60;

/// Claude Code's prompt-cache lifetime for a turn of `kind`, as environment
/// pairs: an hour for the PM and a member's desk, whose turns come minutes
/// apart in one long conversation; nothing (Claude Code's 5 minutes on an API
/// key) for lanes, whose calls come seconds apart. Subagents keep 5 minutes
/// either way: they are short and done before a cache would matter.
///
/// `idle` is how long the member has been without a turn, in seconds, by the
/// wall clock, and `last_short` whether its last turn's cache was written for
/// 5 minutes (`None`/`false` when unknown). Past [`CACHE_HOUR`] the cache is
/// gone and this turn writes the whole conversation again whatever its
/// lifetime; the next wake rarely comes in the window where an hour's write
/// would pay, so that turn keeps 5 minutes. A turn within [`CACHE_SHORT`] of
/// one that kept 5 minutes keeps 5 minutes too: an hour's request would not
/// read what that one wrote. Every other PM and desk turn keeps the hour.
pub fn cache_ttl(kind: TurnKind, idle: Option<u64>, last_short: bool) -> Vec<(String, String)> {
    match kind {
        TurnKind::Pm | TurnKind::PmWake | TurnKind::Desk => {
            let short = idle.is_some_and(|s| s > CACHE_HOUR || (last_short && s <= CACHE_SHORT));
            vec![
                ("CLAUDE_CODE_PROMPT_CACHE_TTL".to_owned(), if short { "5m" } else { "1h" }.to_owned()),
                ("CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL".to_owned(), "5m".to_owned()),
            ]
        }
        TurnKind::Ticket | TurnKind::Design | TurnKind::DesignQa => Vec::new(),
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
    fn the_ticket_env_matches_the_header() {
        assert_eq!(ticket_env(30), ("EQUIPE_TICKET".to_owned(), "T-30".to_owned()));
        assert!(headers("m", "frontend", Some(30), TurnKind::Ticket).contains(&format!("x-equipe-ticket: {}", ticket_env(30).1)));
    }

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
    fn only_sparse_conversations_keep_the_cache_an_hour() {
        for kind in [TurnKind::Pm, TurnKind::PmWake, TurnKind::Desk] {
            for (idle, last_short) in [(None, false), (None, true), (Some(0), false), (Some(20 * 60), false), (Some(CACHE_HOUR), false), (Some(CACHE_SHORT + 1), true), (Some(CACHE_HOUR), true)] {
                let env = cache_ttl(kind, idle, last_short);
                assert_eq!(ttl(&env).as_deref(), Some("1h"), "{kind:?} after {idle:?}, last short: {last_short}");
                assert!(env.contains(&("CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL".to_owned(), "5m".to_owned())), "{kind:?}: subagents stay on 5 minutes");
            }
        }
        for kind in [TurnKind::Ticket, TurnKind::Design, TurnKind::DesignQa] {
            assert!(cache_ttl(kind, None, false).is_empty(), "{kind:?}: lanes call every few seconds and keep Claude Code's 5 minutes");
            assert!(cache_ttl(kind, Some(10 * CACHE_HOUR), true).is_empty(), "{kind:?}");
        }
    }

    fn ttl(env: &[(String, String)]) -> Option<String> {
        env.iter().find(|(k, _)| k == "CLAUDE_CODE_PROMPT_CACHE_TTL").map(|(_, v)| v.clone())
    }

    #[test]
    fn a_conversation_back_after_its_cache_is_gone_writes_it_at_the_5_minute_price() {
        for kind in [TurnKind::Pm, TurnKind::PmWake, TurnKind::Desk] {
            for (idle, last_short) in [(CACHE_HOUR + 1, false), (3 * CACHE_HOUR, true), (30 * CACHE_HOUR, false)] {
                let env = cache_ttl(kind, Some(idle), last_short);
                assert_eq!(ttl(&env).as_deref(), Some("5m"), "{kind:?} after {idle}s");
                assert!(env.contains(&("CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL".to_owned(), "5m".to_owned())), "{kind:?}");
            }
        }
    }

    #[test]
    fn a_wake_soon_after_a_5_minute_turn_stays_on_5_minutes_to_read_it() {
        for idle in [0, 90, CACHE_SHORT] {
            assert_eq!(ttl(&cache_ttl(TurnKind::PmWake, Some(idle), true)).as_deref(), Some("5m"), "{idle}s after a 5-minute turn");
            assert_eq!(ttl(&cache_ttl(TurnKind::PmWake, Some(idle), false)).as_deref(), Some("1h"), "{idle}s after an hour's turn");
        }
    }

    #[test]
    fn the_tools_a_member_works_with_stay() {
        for kept in ["Bash", "Read", "Edit", "Write", "Glob", "Grep", "Task", "Agent", "Monitor", "TaskStop", "SendMessage", "ToolSearch", "Skill", "WebSearch", "WebFetch"] {
            assert!(!UNUSED_TOOLS.contains(&kept), "{kept}");
        }
    }

    #[test]
    fn output_caps_are_below_claude_codes_defaults() {
        let caps = output_caps();
        assert!(caps[0].1.parse::<u32>().unwrap() < 30_000);
        assert!(caps[1].1.parse::<u32>().unwrap() < 25_000);
    }
}
