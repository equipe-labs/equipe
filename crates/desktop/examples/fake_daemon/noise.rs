//! `FAKE_NOISE=1`: the engine's own noise in the designer's conversation, as
//! the person once saw it — Claude Code's word that a command left in the
//! background ended (a `<task-notification>` put in as a user turn, once done
//! with its output file, once failed), a `<system-reminder>` turn, and a
//! foreground `sleep` its harness refused (`Blocked: …`). None of it is the
//! person's: the lines are quiet ones of the designer's, the refusal grey.

use equipe_core::protocol::{Entry, ToolOutcome};

pub fn on() -> bool {
    std::env::var("FAKE_NOISE").is_ok_and(|v| v == "1")
}

const DONE: &str = "<task-notification>\n<task-id>b16yoo5kk</task-id>\n<tool-use-id>toolu_01Hq7d2kQZ</tool-use-id>\n<output-file>/private/tmp/claude-501/-Users-abel-Abel/tasks/b16yoo5kk.output</output-file>\n<status>completed</status>\n<summary>Background command \"Render the onboarding screens\" completed (exit code 0)</summary>\n</task-notification>\nRead the output file to retrieve the result: /private/tmp/claude-501/-Users-abel-Abel/tasks/b16yoo5kk.output";

const FAILED: &str = "<task-notification>\n<task-id>bq83mm1zx</task-id>\n<tool-use-id>toolu_01Pz9Lr0Wb</tool-use-id>\n<status>failed</status>\n<summary>Background command \"Contrast check\" failed with exit code 1</summary>\n</task-notification>";

const BLOCKED: &str = "<tool_use_error>Blocked: sleep 240 followed by: ls shots/. To wait for a condition, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`). To wait for a command you started, use run_in_background: true. Do not chain shorter sleeps to work around this block.</tool_use_error>";

pub fn history() -> Vec<Entry> {
    let run = |target: &str, label: &str, outcome: Option<ToolOutcome>, background: bool| Entry::Tool {
        tool: "Bash".into(),
        target: Some(target.into()),
        outcome: outcome.map(Box::new),
        label: Some(label.into()),
        background,
    };
    let ok = |output: &str| ToolOutcome { ok: true, exit: Some(0), ms: Some(900), output: output.into(), skipped: 0, diff: None };
    vec![
        Entry::User { text: "Render the three onboarding screens and show me".into(), kept: Vec::new() },
        run("npm run render -- onboarding", "Render the onboarding screens", Some(ok("Command running in background with ID: b16yoo5kk.")), true),
        run("sleep 240 && ls shots/", "Wait for the render to finish", Some(ToolOutcome { ok: false, exit: None, ms: Some(3), output: BLOCKED.into(), skipped: 0, diff: None }), false),
        Entry::Reply { text: "The render is running in the background. I'll show you as soon as it's done.".into() },
        Entry::User { text: DONE.into(), kept: Vec::new() },
        run("ls shots/onboarding", "Check the render", Some(ok("1-welcome.png\n2-goals.png\n3-ready.png")), false),
        run("npm run contrast -- shots/onboarding", "Contrast check", Some(ok("Command running in background with ID: bq83mm1zx.")), true),
        Entry::User { text: FAILED.into(), kept: Vec::new() },
        Entry::User { text: "<system-reminder>\nThe task tools haven't been used recently.\n</system-reminder>".into(), kept: Vec::new() },
        Entry::Reply { text: "All three screens are rendered. The contrast check caught the secondary text on the second screen — I'll make its colour a step darker and look again.".into() },
    ]
}
