//! A ceiling on what equipe's own tools hand a member's engine in one answer.
//!
//! A tool answer goes into the conversation and is re-read on every later
//! model call, so one 100k-character board dump or desktop snapshot early in a
//! turn is paid for again with each step after it. Every answer of equipe's
//! MCP servers (the board, the desktop, the phone, the design tools) passes
//! through [`capped`] on its way out: past [`MAX_TOOL_CHARS`] (about
//! [`MAX_TOOL_TOKENS`] tokens) it is cut at a line and ends with what to call
//! to see the rest, so nothing is lost — only fetched when needed.
//!
//! The board is paged by its own arguments instead (`board::render_page`), so
//! its tail names a call that works. Turned off for a squad with
//! `equipe context-config --tool-caps off` (`Journal::context_tool_caps`).

/// About how many tokens one tool answer may hold: long enough for a large
/// board page or a whole app's tree, short enough that a single answer can't
/// fill a conversation.
pub const MAX_TOOL_TOKENS: usize = 10_000;

/// Characters per token the cap assumes (English text and code run 3 to 4).
const CHARS_PER_TOKEN: usize = 4;

/// The ceiling in characters.
pub const MAX_TOOL_CHARS: usize = MAX_TOOL_TOKENS * CHARS_PER_TOKEN;

/// About how many tokens `text` is.
pub fn tokens_of(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// What to call to see more of a tool's cut answer.
fn more(tool: &str) -> String {
    match tool {
        "board" => "call `board` with `after` set to the last ticket number shown, or with `status` to list one status".to_owned(),
        "snapshot" | "elements" => format!("call `{tool}` again with `skeleton: true` for a shallow map, or with `root` set to one ref of the branch you need (and a smaller `depth`)"),
        "find" => "call `find` with a narrower `role`, `name` or `text`, or a smaller `limit`".to_owned(),
        "windows" | "apps" | "notifications" => format!("call `{tool}` again for one app or window"),
        "design_tools" | "system_tools" => format!("call `{tool}` again with `args` for one narrower command"),
        "signals" => "call `signals` with `name` set to one signal".to_owned(),
        "metrics" => "call `metrics` with `metric` or `project` set, or a smaller `days`".to_owned(),
        other => format!("call `{other}` again with narrower arguments"),
    }
}

/// `text` as it goes to the engine: whole when it fits [`MAX_TOOL_CHARS`],
/// else cut at the last line that fits, with a tail naming what was left out
/// and what to call to see it.
pub fn capped(tool: &str, text: &str) -> String {
    capped_at(tool, text, MAX_TOOL_CHARS)
}

/// [`capped`] with another ceiling, in characters.
pub fn capped_at(tool: &str, text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_owned();
    }
    let mut end = text.char_indices().nth(max).map_or(text.len(), |(at, _)| at);
    // Back to the end of a line, unless that gives up most of what fits.
    if let Some(line) = text[..end].rfind('\n').filter(|at| text[..*at].chars().count() >= max * 4 / 5) {
        end = line;
    }
    let shown = text[..end].trim_end();
    let left = total - shown.chars().count();
    format!(
        "{shown}\n\n[truncated — {left} more characters (about {}k tokens) are not shown. To see more: {}.]",
        tokens_of(&text[shown.len()..]).div_ceil(1000).max(1),
        more(tool)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_that_fits_goes_through_untouched() {
        let text = "T-1 [todo] one\nT-2 [todo] two\n";
        assert_eq!(capped("board", text), text);
    }

    #[test]
    fn a_long_answer_is_cut_at_a_line_with_a_tail_that_says_how_to_see_more() {
        let line = "e12 button \"Save the document\" (focused)\n";
        let big = line.repeat(10_000);
        assert!(tokens_of(&big) > 90_000);
        let out = capped("snapshot", &big);
        assert!(tokens_of(&out) <= MAX_TOOL_TOKENS + 100, "{} tokens", tokens_of(&out));
        assert!(out.contains("truncated"), "{out}");
        assert!(out.contains("`snapshot`") && out.contains("skeleton"), "the tail names the call");
        let kept = out.split("\n\n[truncated").next().unwrap();
        assert!(kept.lines().all(|l| l == line.trim_end()), "cut on a line");
    }

    #[test]
    fn it_cuts_on_a_character_boundary() {
        let big = "ö".repeat(MAX_TOOL_CHARS + 500);
        let out = capped("anything", &big);
        assert!(out.starts_with('ö') && out.contains("narrower arguments"));
        assert!(out.chars().filter(|c| *c == 'ö').count() <= MAX_TOOL_CHARS);
    }
}
