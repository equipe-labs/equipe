//! The second look: before a designer hands a screen in, a reviewer who saw
//! none of the work being made judges it — started fresh, with nothing of
//! the designer's conversation, given only the pictures drawn at each width,
//! the brief and its acceptance criteria, and the design system. It answers
//! pass, or the fixes it wants, and its verdict is kept on the screen's
//! check record (`lint::record`), where the hand-in looks for it.
//!
//! For a new screen it looks at the first screenful alone first, before the
//! rest is built on it.

use std::path::{Path, PathBuf};

use equipe_protocol::Engine;

use super::lint::record::{Fix, Verdict};

/// The line a second look ends with, for screens that show it.
pub const MARK: &str = "equipe-second-look:";

/// What the reviewer is given — and nothing else.
#[derive(Debug, Clone, Default)]
pub struct Ask {
    pub screen: String,
    /// The pictures, one per width, in order.
    pub pictures: Vec<(u32, PathBuf)>,
    /// The brief: `brand.md`, or the ticket's.
    pub brief: Option<String>,
    /// What the screen must do, as the ticket says it.
    pub criteria: Option<String>,
    /// The system's rules in prose (`guidelines.md`), cut to a length.
    pub guidelines: Option<String>,
    /// The tokens file, to open if it needs a value.
    pub tokens: Option<PathBuf>,
    /// The first screenful alone.
    pub first: bool,
    /// What the lint found and the designer kept on purpose, so it isn't asked again.
    pub kept: Vec<(String, String)>,
}

/// How much of the guidelines it is given.
const GUIDELINES_CUT: usize = 12_000;

pub fn cut(text: &str) -> String {
    if text.len() <= GUIDELINES_CUT {
        return text.to_owned();
    }
    let mut end = GUIDELINES_CUT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…", &text[..end])
}

const ROLE: &str = "You are reviewing a product screen you have never seen before. You did not make it, you were not \
there while it was made, and you owe its designer nothing: judge only what the pictures show against the brief and \
the design system below. Open each picture with your Read tool before you answer.\n\
\n\
Look for what code can't check: does the screen do what the brief and its acceptance criteria ask, and is the first \
thing the eye lands on the thing that matters most? Is the hierarchy clear at each width, or does a narrow width \
crowd, stack badly or bury the primary action? Is the spacing on one rhythm, are things aligned to a grid, does the \
type read as one system? Does it use the system's components and roles, or invent its own? Is every word real \
copy — no placeholder, no invented figures, customers or quotes? Does it look like this product, or like any \
generated template?";

const FIRST: &str = "These pictures are only the first screenful of a new screen, before the rest of it is built. Judge \
whether this opening is right to build on: what it leads with, its hierarchy, its density and its tone. Don't ask \
for what is below the fold.";

const ANSWER: &str = "Answer with one JSON object and nothing after it:\n\
{\"verdict\": \"pass\" | \"fix\", \"summary\": \"two sentences on the whole\", \"fixes\": [{\"where\": \"the region \
(top bar, second card, the form at 360)\", \"what\": \"what to change\", \"why\": \"what it costs the person now\"}]}\n\
Say \"pass\" only when nothing needs fixing before a person sees it; polish you'd merely like is not a fix. Worst \
fix first, at most eight, each tied to a region and, where it differs, a width. Write in the brief's language.";

/// The prompt: the reviewer's role, what it is given, and how to answer.
pub fn prompt(ask: &Ask) -> String {
    let mut out = String::from(ROLE);
    if ask.first {
        out.push_str("\n\n");
        out.push_str(FIRST);
    }
    out.push_str(&format!("\n\nThe screen: {}\nIts pictures, one per width:", ask.screen));
    for (width, path) in &ask.pictures {
        out.push_str(&format!("\n- {width}px: {}", path.display()));
    }
    if let Some(criteria) = ask.criteria.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n\nWhat it must do (the acceptance criteria):\n<criteria>\n{criteria}\n</criteria>"));
    }
    if let Some(brief) = ask.brief.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        out.push_str(&format!("\n\nThe brief:\n<brief>\n{brief}\n</brief>"));
    }
    if let Some(g) = ask.guidelines.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
        out.push_str(&format!("\n\nThe design system's rules:\n<guidelines>\n{}\n</guidelines>", cut(g)));
    }
    if let Some(tokens) = &ask.tokens {
        out.push_str(&format!("\nIts tokens, if you need a value: {}", tokens.display()));
    }
    if !ask.kept.is_empty() {
        out.push_str("\n\nKept on purpose, with the person's reason (don't ask for these again):");
        for (rule, why) in &ask.kept {
            out.push_str(&format!("\n- {rule}: {why}"));
        }
    }
    out.push_str("\n\n");
    out.push_str(ANSWER);
    out
}

/// The program and its arguments for one fresh look, its answer read from
/// `answer` (Codex writes its last message there; Claude prints it).
pub fn command(engine: Engine, ask: &Ask, answer: &Path) -> (String, Vec<String>) {
    let text = prompt(ask);
    match engine {
        Engine::Codex => {
            let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"].map(String::from).into();
            for (_, image) in &ask.pictures {
                args.extend(["--image".into(), image.display().to_string()]);
            }
            args.extend(["--output-last-message".into(), answer.display().to_string(), text]);
            (crate::engines::codex_binary(), args)
        }
        Engine::Claude => {
            // A new session: no `--resume`, no memory of anyone's; it may only read.
            let args = ["-p", &text, "--allowedTools", "Read", "--disallowedTools", "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch,Agent,Task"].map(String::from).into();
            (equipe_providers::claude_code::DEFAULT_BINARY.to_owned(), args)
        }
    }
}

/// The verdict in an answer: its last JSON object with a `verdict`.
pub fn verdict_of(answer: &str) -> Option<Verdict> {
    let mut found = None;
    for (at, _) in answer.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&answer[at..]).into_iter::<serde_json::Value>();
        if let Some(Ok(value)) = stream.next()
            && let Some(v) = value.get("verdict").and_then(serde_json::Value::as_str)
        {
            let fixes: Vec<Fix> = value.get("fixes").and_then(|f| serde_json::from_value(f.clone()).ok()).unwrap_or_default();
            let pass = v.trim().eq_ignore_ascii_case("pass") && fixes.is_empty();
            found = Some(Verdict { pass, fixes, summary: value.get("summary").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(), ..Verdict::default() });
        }
    }
    found
}

/// The verdict as a person reads it.
pub fn text(v: &Verdict) -> String {
    let mut out = format!("{}{}\n", if v.pass { "Pass" } else { "Fix" }, if v.summary.is_empty() { String::new() } else { format!(" — {}", v.summary) });
    for f in &v.fixes {
        out.push_str(&format!("- {}: {}{}\n", f.place, f.what, if f.why.is_empty() { String::new() } else { format!(" ({})", f.why) }));
    }
    out
}

/// The line screens read: [`MARK`], the screen and the verdict as JSON.
pub fn mark_line(screen: &str, v: &Verdict) -> String {
    format!("{MARK} {}", serde_json::json!({ "screen": screen, "verdict": v }))
}

/// A second look in a run's output, if it ends with one: the screen and its verdict.
pub fn in_output(output: &str) -> Option<(String, Verdict)> {
    let line = output.lines().rev().find(|l| l.starts_with(MARK))?;
    let value: serde_json::Value = serde_json::from_str(line[MARK.len()..].trim()).ok()?;
    Some((value.get("screen")?.as_str()?.to_owned(), serde_json::from_value(value.get("verdict")?.clone()).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask() -> Ask {
        Ask {
            screen: "settings/notifications".into(),
            pictures: vec![(360, "design/checks/settings/notifications/360.png".into()), (1280, "design/checks/settings/notifications/1280.png".into())],
            brief: Some("Calm, plain.".into()),
            criteria: Some("- The person turns the digest off in one step".into()),
            guidelines: Some("x".repeat(GUIDELINES_CUT + 50)),
            tokens: Some("design/tokens.json".into()),
            first: false,
            kept: vec![("tracked-caps".into(), "the brand's labels".into())],
        }
    }

    #[test]
    fn the_reviewer_gets_only_the_pictures_the_brief_the_criteria_and_the_system() {
        let p = prompt(&ask());
        assert!(p.contains("never seen before") && p.contains("you were not there while it was made"));
        for must in ["360px: design/checks/settings/notifications/360.png", "1280px:", "<criteria>", "turns the digest off", "<brief>", "<guidelines>", "design/tokens.json", "tracked-caps: the brand's labels", "\"verdict\": \"pass\" | \"fix\""] {
            assert!(p.contains(must), "{must}");
        }
        assert!(p.len() < GUIDELINES_CUT + 4000, "the guidelines are cut");
        assert!(!p.contains("first screenful of a new screen"));
        let first = prompt(&Ask { first: true, ..ask() });
        assert!(first.contains("first screenful of a new screen"));
    }

    #[test]
    fn a_fresh_reviewer_only_reads() {
        let (program, args) = command(Engine::Claude, &ask(), Path::new("a.md"));
        assert!(program.contains("claude"));
        assert!(!args.iter().any(|a| a == "--resume" || a == "--continue"), "a fresh session");
        let denied = args.iter().position(|a| a == "--disallowedTools").map(|i| args[i + 1].clone()).unwrap();
        assert!(denied.contains("Bash") && denied.contains("Write") && denied.contains("Agent"));
        let (_, args) = command(Engine::Codex, &ask(), Path::new("a.md"));
        assert!(args.windows(2).any(|w| w[0] == "--sandbox" && w[1] == "read-only") && args.iter().filter(|a| *a == "--image").count() == 2);
    }

    #[test]
    fn the_verdict_is_read_from_its_answer_and_a_pass_with_fixes_is_a_fix() {
        let v = verdict_of("I looked.\n```json\n{\"verdict\": \"fix\", \"summary\": \"Crowded at 360.\", \"fixes\": [{\"where\": \"form at 360\", \"what\": \"stack the buttons\", \"why\": \"Save is cut off\"}]}\n```").unwrap();
        assert!(!v.pass && v.fixes.len() == 1 && v.fixes[0].place == "form at 360");
        assert!(verdict_of("{\"verdict\": \"pass\", \"summary\": \"Clear.\", \"fixes\": []}").unwrap().pass);
        assert!(!verdict_of("{\"verdict\": \"pass\", \"fixes\": [{\"where\": \"x\", \"what\": \"y\"}]}").unwrap().pass);
        assert!(verdict_of("no json here").is_none());
        let line = mark_line("a/b", &v);
        assert_eq!(in_output(&format!("…\n{line}\n")), Some(("a/b".to_owned(), v.clone())));
        assert!(text(&v).starts_with("Fix — Crowded at 360.\n- form at 360: stack the buttons (Save is cut off)"));
    }
}
