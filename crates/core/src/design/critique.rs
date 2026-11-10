//! `equipe design critique`: the designer's pictures go to a fresh reviewer
//! for a sceptical critique — eyes that saw none of the work being made. The
//! other engine when it can take it, else a fresh session of the member's
//! own, as the squad sets it (`reviewer`). The critic only reads: Codex in
//! its read-only sandbox, Claude with its reading tools alone.

use std::path::{Path, PathBuf};

use equipe_protocol::Engine;

use crate::roles::other_engine;

/// The engine the caller runs on, from what each engine sets in the shells it
/// opens: Codex its thread, Claude Code `CLAUDECODE`. None from a person's
/// own terminal.
pub fn caller_engine(env: impl Fn(&str) -> Option<String>) -> Option<Engine> {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
    if set("CODEX_THREAD_ID") || set("CODEX_SANDBOX") {
        Some(Engine::Codex)
    } else if set("CLAUDECODE") {
        Some(Engine::Claude)
    } else {
        // A member's command run outside its engine's sandbox loses the
        // engine's own variables; equipe's, set for every member, stays.
        match env(MEMBER_ENGINE_ENV).as_deref().map(str::trim) {
            Some("codex") => Some(Engine::Codex),
            Some("claude") => Some(Engine::Claude),
            _ => None,
        }
    }
}

/// Set in a member's commands' environment: the engine the member runs on.
pub const MEMBER_ENGINE_ENV: &str = "EQUIPE_MEMBER_ENGINE";

/// The value [`MEMBER_ENGINE_ENV`] holds for `engine`.
pub fn member_engine_value(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// What's said when nobody knows who is calling and `engine` was taken for
/// `what`: it is never chosen unsaid.
pub fn unknown_caller(engine: Engine, what: &str) -> String {
    format!(
        "No engine was named and none is known for whoever runs this, so the {what} runs on {}. Name one with `--engine claude` or `--engine codex`.",
        engine.label()
    )
}

/// When an engine couldn't be reached because the caller's sandbox has no
/// network (`said` is what it printed): what the member does next.
pub fn no_network(said: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let sandboxed = ["CODEX_SANDBOX", "CODEX_SANDBOX_NETWORK_DISABLED"].iter().any(|n| env(n).is_some_and(|v| !v.is_empty()));
    let lower = said.to_lowercase();
    let unreachable = ["enotfound", "can't reach the api", "could not resolve host", "network error", "error sending request", "connection refused", "dns"].iter().any(|w| lower.contains(w));
    (sandboxed && unreachable).then(|| {
        "It couldn't reach its engine: your sandbox has no network. Run the same command again outside the sandbox — ask for it, and the person allows or refuses. If that isn't allowed, don't try again: go on, and say in your hand-in that this check couldn't run and why.".to_owned()
    })
}

/// The engine the caller isn't on — a brief's second opinion
/// (`equipe brief critique`); a design review asks [`super::reviewer`]. From
/// a person's terminal, Codex — the designer works on Claude unless moved.
pub fn critic_engine(caller: Option<Engine>) -> Engine {
    caller.map_or(Engine::Codex, other_engine)
}

/// What the critic is given.
#[derive(Debug, Default, Clone)]
pub struct Ask {
    /// The pictures to critique.
    pub pictures: Vec<PathBuf>,
    /// The reference it should stand beside.
    pub references: Vec<PathBuf>,
    /// The round before, to say whether the revision lost the whole.
    pub before: Vec<PathBuf>,
    /// The brief (`brand.md`): reference, locked and free, the thesis.
    pub brief: Option<String>,
    /// What the work is for, in the designer's words.
    pub goal: Option<String>,
}

impl Ask {
    /// Every picture the critic opens, in the order the prompt names them.
    pub fn images(&self) -> Vec<&Path> {
        self.pictures.iter().chain(&self.references).chain(&self.before).map(PathBuf::as_path).collect()
    }
}

/// The rubric: the whole first, then the details, each finding on a region.
const RUBRIC: &str = "You are a sceptical senior product designer critiquing another designer's work. Find faults; do not \
praise, and do not soften. You did not make this, and you are not grading effort.\n\
\n\
First the whole. At a three-second glance, and again squinting: where does the eye go first, second, third — and is \
that the right order? Does it breathe — whitespace, rhythm, a clear grid — or is it cramped or scattered? Next to the \
reference, does it look finished and expensive, or like a template? Hide the logo: could this be another company?\n\
\n\
Then the details: hierarchy and density; alignment and spacing on one scale; type (scale, line length, set for \
the product's language — a family with its script, its line breaking, right to left where it reads so, no orphans — \
numbers in tabular figures); colour roles and contrast (text under WCAG AA \
is a blocker); brand fidelity against the brief; states and real copy (placeholder text, missing empty or error \
states); the stock looks (cream page with a serif, Inter with an indigo gradient, three equal cards, emoji as icons).\n\
\n\
When a round before is given: is the revision better as a whole? A fix that loses the whole is worse than the flaw \
it fixed — say so plainly.\n\
\n\
Answer in English, whatever language the brief, the goal or the work is in — the critique is kept in the project's \
design folder, and whoever passes it on to the person says it in their language — in this shape:\n\
## Whole\n\
Two or three sentences on the impression, against the reference.\n\
## Findings\n\
One line each, worst first: `[blocker|polish] <picture> · <region> — what is wrong — what to do`. A blocker must \
be fixed before the person sees it; polish should be. Name the region (top bar, hero headline, second card, \
footer…), never just \"the page\".\n\
## Verdict\n\
`revise` or `show` — show only when no blocker is left — and the one change that would help most.";

/// The prompt for the critic: the rubric, the brief, and what each picture is.
pub fn prompt(ask: &Ask) -> String {
    let mut out = String::from(RUBRIC);
    let list = |title: &str, paths: &[PathBuf], out: &mut String| {
        if paths.is_empty() {
            return;
        }
        out.push_str(&format!("\n\n{title}:"));
        for p in paths {
            out.push_str(&format!("\n- {}", p.display()));
        }
    };
    list("The work to critique", &ask.pictures, &mut out);
    list("The reference it should stand beside", &ask.references, &mut out);
    list("The round before this one", &ask.before, &mut out);
    if let Some(goal) = ask.goal.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
        out.push_str(&format!("\n\nWhat it is for: {goal}"));
    }
    if let Some(brief) = ask.brief.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        out.push_str(&format!("\n\nThe brief (brand.md):\n<brief>\n{brief}\n</brief>"));
    }
    out
}

/// The program and its arguments, to run the critic once and read its answer
/// from `answer` (Codex writes its last message there; Claude prints it).
pub fn command(engine: Engine, ask: &Ask, answer: &Path) -> (String, Vec<String>) {
    let text = prompt(ask);
    match engine {
        Engine::Codex => {
            let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"].map(String::from).into();
            for image in ask.images() {
                args.extend(["--image".into(), image.display().to_string()]);
            }
            args.extend(["--output-last-message".into(), answer.display().to_string(), text]);
            (crate::engines::codex_binary(), args)
        }
        Engine::Claude => {
            // Claude opens the pictures itself with Read; it may do nothing else.
            let args = ["-p", &text, "--allowedTools", "Read", "--disallowedTools", "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch"].map(String::from).into();
            (equipe_providers::claude_code::DEFAULT_BINARY.to_owned(), args)
        }
    }
}

/// Where a critique is kept: `design/critiques/<stamp>.md`, beside the work.
pub fn kept_at(design: &Path, stamp: &str) -> PathBuf {
    design.join("critiques").join(format!("{stamp}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn the_other_engine_critiques_what_one_engine_drew() {
        assert_eq!(caller_engine(env(&[("CLAUDECODE", "1")])), Some(Engine::Claude));
        assert_eq!(caller_engine(env(&[("CODEX_THREAD_ID", "t-1")])), Some(Engine::Codex));
        assert_eq!(caller_engine(env(&[("CLAUDECODE", "1"), ("CODEX_SANDBOX", "seatbelt")])), Some(Engine::Codex), "a Codex shell started from Claude Code is Codex's");
        assert_eq!(caller_engine(env(&[("CLAUDECODE", "")])), None);
        assert_eq!(critic_engine(Some(Engine::Claude)), Engine::Codex);
        assert_eq!(critic_engine(Some(Engine::Codex)), Engine::Claude);
        assert_eq!(critic_engine(None), Engine::Codex, "from a person's terminal, the designer is on Claude");
    }

    #[test]
    fn a_members_command_outside_its_engines_sandbox_still_knows_its_engine() {
        // A Codex member's command run outside the sandbox has none of Codex's
        // own variables; equipe's says whose it is.
        assert_eq!(caller_engine(env(&[(MEMBER_ENGINE_ENV, "codex")])), Some(Engine::Codex));
        assert_eq!(caller_engine(env(&[(MEMBER_ENGINE_ENV, "claude")])), Some(Engine::Claude));
        assert_eq!(caller_engine(env(&[(MEMBER_ENGINE_ENV, "codex"), ("CLAUDECODE", "1")])), Some(Engine::Claude), "the engine's own word first");
        assert_eq!(caller_engine(env(&[(MEMBER_ENGINE_ENV, "other")])), None);
        assert_eq!(member_engine_value(Engine::Codex), "codex");
        assert_eq!(member_engine_value(Engine::Claude), "claude");
        assert!(unknown_caller(Engine::Claude, "second look").contains("--engine"), "never Claude unsaid");
    }

    #[test]
    fn a_critic_the_sandbox_cuts_off_from_the_network_says_what_to_do() {
        let said = "API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)";
        let sandboxed = env(&[("CODEX_SANDBOX", "seatbelt")]);
        let told = no_network(said, &sandboxed).expect("the sandbox has no network");
        assert!(told.contains("outside the sandbox") && told.contains("hand-in"), "{told}");
        assert!(no_network("stream error: network error: error sending request", &env(&[("CODEX_SANDBOX_NETWORK_DISABLED", "1")])).is_some());
        assert!(no_network(said, &env(&[])).is_none(), "not in a sandbox: the network itself is down");
        assert!(no_network("Invalid API key", &sandboxed).is_none(), "another failure is said as it is");
    }

    fn ask() -> Ask {
        Ask {
            pictures: vec!["shots/home-1280.png".into(), "shots/home-390-dark.png".into()],
            references: vec!["design/references/linear/home.png".into()],
            before: vec!["shots/round1/home-1280.png".into()],
            brief: Some("Reference: Linear's home — its calm density.\n## Locked\nnavy #0B1F3A".into()),
            goal: Some("A clinic marketing site's home".into()),
        }
    }

    #[test]
    fn the_critic_judges_the_whole_first_then_each_region_against_the_brief() {
        let p = prompt(&ask());
        let whole = p.find("three-second glance").expect("the whole first");
        assert!(p.find("Then the details").unwrap() > whole);
        for part in ["squinting", "breathe", "expensive", "Hide the logo", "loses the whole", "[blocker|polish]", "region", "`revise` or `show`"] {
            assert!(p.contains(part), "{part}");
        }
        for path in ["shots/home-1280.png", "design/references/linear/home.png", "shots/round1/home-1280.png"] {
            assert!(p.contains(path), "{path} named");
        }
        assert!(p.contains("<brief>\nReference: Linear's home"));
        assert!(p.contains("A clinic marketing site's home"));
        let bare = prompt(&Ask { pictures: vec!["a.png".into()], ..Ask::default() });
        assert!(!bare.contains("reference it should stand beside") && !bare.contains("<brief>") && !bare.contains("round before this one"));
        // Kept in the project's design folder: in English, whatever the brief or the goal is written in.
        assert!(p.contains("Answer in English, whatever language the brief, the goal or the work is in") && p.contains("kept in the project's design folder"));
    }

    #[test]
    fn codex_critiques_read_only_with_every_picture_attached() {
        let (bin, args) = command(Engine::Codex, &ask(), Path::new("/tmp/answer.md"));
        assert!(bin.ends_with("codex"));
        assert_eq!(&args[..3], ["exec", "--sandbox", "read-only"]);
        let images: Vec<&str> = args.windows(2).filter(|w| w[0] == "--image").map(|w| w[1].as_str()).collect();
        assert_eq!(images, ["shots/home-1280.png", "shots/home-390-dark.png", "design/references/linear/home.png", "shots/round1/home-1280.png"]);
        let at = args.iter().position(|a| a == "--output-last-message").unwrap();
        assert_eq!(args[at + 1], "/tmp/answer.md");
        assert!(args.last().unwrap().starts_with("You are a sceptical"), "the prompt last");
        assert!(!args.iter().any(|a| a.contains("danger") || a.contains("bypass")));
    }

    #[test]
    fn claude_critiques_with_its_reading_tools_alone() {
        let (bin, args) = command(Engine::Claude, &ask(), Path::new("/tmp/answer.md"));
        assert_eq!(bin, "claude");
        assert_eq!(args[0], "-p");
        assert!(args[1].contains("shots/home-1280.png"), "it opens the pictures by path");
        let allowed = args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(args[allowed + 1], "Read");
        let denied = args.iter().position(|a| a == "--disallowedTools").unwrap();
        for tool in ["Bash", "Edit", "Write"] {
            assert!(args[denied + 1].contains(tool));
        }
        assert!(!args.iter().any(|a| a == "--bare" || a.contains("dangerously")), "never --bare, never past permissions");
    }

    #[test]
    fn a_critique_is_kept_beside_the_work() {
        assert_eq!(kept_at(Path::new("design"), "20261103-0912"), PathBuf::from("design/critiques/20261103-0912.md"));
    }
}
