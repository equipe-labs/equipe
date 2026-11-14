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

use super::lint::record::{Fix, Question, Verdict};

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
    /// The best products' screens for the patterns it uses (the project's
    /// `design/references/`), drawn at about the same width: the tier it is
    /// held to.
    pub references: Vec<PathBuf>,
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
generated template? Each failure of the taste checklist is a fix too: {checklist}.\n\
\n\
{kinds} Each failure of it is a fix too.\n\
\n\
Ask only for facts the brief, the project's files or the design system give. When the screen needs a fact none of \
them has — a name, a label, a number, a date — that is not a fix and not a reason to withhold a pass: put it in \
\"questions\", with neutral wording the screen can use until someone answers.";

/// With references: the tier question.
fn tier(references: &[PathBuf]) -> String {
    let mut out = format!(
        "Beside it, the references: the best products' screens for the patterns it uses, drawn at about the same width. \
Is the screen in the same quality tier? Below it is a fix: name each concrete difference — {} — tied to a region.\n\
The references:",
        super::critique::TIER_GAPS
    );
    for path in references {
        out.push_str(&format!("\n- {}", path.display()));
    }
    out
}

const FIRST: &str = "These pictures are only the first screenful of a new screen, before the rest of it is built. Judge \
whether this opening is right to build on: what it leads with, its hierarchy, its density and its tone. Don't ask \
for what is below the fold.";

const ANSWER: &str = "Answer with one JSON object and nothing after it:\n\
{\"verdict\": \"pass\" | \"fix\", \"summary\": \"two sentences on the whole\", \"tier\": \"same\" | \"below\" | \"no references\", \"fixes\": [{\"where\": \"the region \
(top bar, second card, the form at 360)\", \"what\": \"what to change\", \"why\": \"what it costs the person now\"}], \
\"questions\": [{\"what\": \"the fact that is missing\", \"wording\": \"what the screen can say without it\"}]}\n\
Say \"pass\" only when nothing needs fixing before a person sees it; polish you'd merely like is not a fix. Worst \
fix first, at most eight, each tied to a region and, where it differs, a width. Write in the brief's language.";

/// The prompt: the reviewer's role, what it is given, and how to answer.
pub fn prompt(ask: &Ask) -> String {
    let mut out = ROLE.replace("{checklist}", super::critique::TASTE_CHECKLIST).replace("{kinds}", super::critique::KIND_CHECKLIST);
    if ask.first {
        out.push_str("\n\n");
        out.push_str(FIRST);
    }
    out.push_str(&format!("\n\nThe screen: {}\nIts pictures, one per width:", ask.screen));
    for (width, path) in &ask.pictures {
        out.push_str(&format!("\n- {width}px: {}", path.display()));
    }
    if !ask.references.is_empty() {
        out.push_str("\n\n");
        out.push_str(&tier(&ask.references));
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
            for image in ask.pictures.iter().map(|(_, p)| p).chain(&ask.references) {
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
            let questions: Vec<Question> = value.get("questions").and_then(|q| serde_json::from_value(q.clone()).ok()).unwrap_or_default();
            let questions: Vec<Question> = questions.into_iter().filter(|q| !q.what.trim().is_empty()).collect();
            // A missing fact is asked, never a fix: a "fix" answer that lists
            // nothing to fix but questions passes. Below its references' tier never does.
            let said = |key: &str| value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().trim().to_owned();
            let tier = said("tier");
            let pass = (v.trim().eq_ignore_ascii_case("pass") || !questions.is_empty()) && fixes.is_empty() && !tier.eq_ignore_ascii_case("below");
            found = Some(Verdict { pass, fixes, questions, summary: said("summary"), tier, ..Verdict::default() });
        }
    }
    found
}

/// The verdict as a person reads it.
pub fn text(v: &Verdict) -> String {
    let mut out = format!("{}{}\n", if v.pass { "Pass" } else { "Fix" }, if v.summary.is_empty() { String::new() } else { format!(" — {}", v.summary) });
    if !v.tier.is_empty() {
        out.push_str(&format!("Tier against the references: {}\n", v.tier));
    }
    for f in &v.fixes {
        out.push_str(&format!("- {}: {}{}\n", f.place, f.what, if f.why.is_empty() { String::new() } else { format!(" ({})", f.why) }));
    }
    if !v.questions.is_empty() {
        out.push_str("Open questions — facts nothing you were given has; not fixes. Use the wording, or ask the person with ask_decision while you carry on:\n");
        for q in &v.questions {
            out.push_str(&format!("- {}{}\n", q.what, if q.wording.is_empty() { String::new() } else { format!(" — meanwhile: \"{}\"", q.wording) }));
        }
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
            references: vec![],
        }
    }

    #[test]
    fn with_references_it_judges_the_tier_and_always_the_taste_checklist() {
        let bare = prompt(&ask());
        assert!(bare.contains(crate::design::critique::TASTE_CHECKLIST) && !bare.contains("{checklist}"), "the checklist, with or without references");
        assert!(bare.contains(crate::design::critique::KIND_CHECKLIST) && !bare.contains("{kinds}"), "the product kind's bar, always");
        assert!(!bare.contains("Beside it, the references"));
        let refs = vec![PathBuf::from("design/references/kpi-tile/a-1280.png"), PathBuf::from("design/references/filter-bar/b-1280.png")];
        let given = Ask { references: refs, ..ask() };
        let p = prompt(&given);
        for must in ["same quality tier", "Below it is a fix", crate::design::critique::TIER_GAPS, "- design/references/kpi-tile/a-1280.png", "\"tier\": \"same\" | \"below\""] {
            assert!(p.contains(must), "{must}");
        }
        let (_, args) = command(Engine::Codex, &given, Path::new("a.md"));
        let images: Vec<&str> = args.windows(2).filter(|w| w[0] == "--image").map(|w| w[1].as_str()).collect();
        assert_eq!(
            images,
            ["design/checks/settings/notifications/360.png", "design/checks/settings/notifications/1280.png", "design/references/kpi-tile/a-1280.png", "design/references/filter-bar/b-1280.png"],
            "the references attached after the work"
        );
        let v = verdict_of("{\"verdict\": \"fix\", \"tier\": \"below\", \"summary\": \"Looser than the references.\", \"fixes\": [{\"where\": \"KPI row\", \"what\": \"tighten to the references' density\"}]}").unwrap();
        assert!(!v.pass && v.tier == "below");
        assert!(!verdict_of("{\"verdict\": \"pass\", \"tier\": \"below\", \"fixes\": []}").unwrap().pass, "below the references' tier is never a pass");
        assert!(verdict_of("{\"verdict\": \"pass\", \"tier\": \"same\", \"fixes\": []}").unwrap().pass);
        assert!(text(&v).contains("Tier against the references: below"), "{}", text(&v));
        let older: Verdict = serde_json::from_str("{\"pass\": true}").unwrap();
        assert!(older.tier.is_empty(), "a verdict kept before tiers reads back");
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

    #[test]
    fn a_fact_nothing_gives_is_asked_as_a_question_and_never_holds_a_pass() {
        let p = prompt(&ask());
        assert!(p.contains("Ask only for facts the brief, the project's files or the design system give") && p.contains("not a fix and not a reason to withhold a pass") && p.contains("\"questions\": [{\"what\""), "{p}");
        let v = verdict_of("{\"verdict\": \"fix\", \"summary\": \"Good, but the save control isn't named.\", \"fixes\": [], \"questions\": [{\"what\": \"what the save control is called\", \"wording\": \"Articles you save appear here.\"}]}").unwrap();
        assert!(v.pass, "only a question: a pass");
        assert_eq!(v.questions, [Question { what: "what the save control is called".into(), wording: "Articles you save appear here.".into() }]);
        let said = text(&v);
        assert!(said.contains("Open questions") && said.contains("not fixes") && said.contains("ask_decision") && said.contains("meanwhile: \"Articles you save appear here.\""), "{said}");
        let both = verdict_of("{\"verdict\": \"fix\", \"fixes\": [{\"where\": \"hero\", \"what\": \"x\"}], \"questions\": [{\"what\": \"y\"}]}").unwrap();
        assert!(!both.pass && both.questions.len() == 1, "a real fix still holds it");
        assert!(verdict_of("{\"verdict\": \"pass\", \"fixes\": []}").unwrap().questions.is_empty());
        let line = mark_line("a/b", &v);
        assert_eq!(in_output(&line).unwrap().1, v, "questions travel with the verdict");
        let older: Verdict = serde_json::from_str("{\"pass\": true, \"fixes\": []}").unwrap();
        assert!(older.questions.is_empty(), "a verdict kept before questions reads back");
    }
}
