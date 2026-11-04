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

const DONE: &str = "<task-notification>\n<task-id>b16yoo5kk</task-id>\n<tool-use-id>toolu_01Hq7d2kQZ</tool-use-id>\n<output-file>/private/tmp/claude-501/-Users-abel-Abel/tasks/b16yoo5kk.output</output-file>\n<status>completed</status>\n<summary>Background command \"온보딩 화면 렌더\" completed (exit code 0)</summary>\n</task-notification>\nRead the output file to retrieve the result: /private/tmp/claude-501/-Users-abel-Abel/tasks/b16yoo5kk.output";

const FAILED: &str = "<task-notification>\n<task-id>bq83mm1zx</task-id>\n<tool-use-id>toolu_01Pz9Lr0Wb</tool-use-id>\n<status>failed</status>\n<summary>Background command \"대비 검사\" failed with exit code 1</summary>\n</task-notification>";

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
        Entry::User { text: "온보딩 세 화면 렌더해서 보여 줘".into() },
        run("npm run render -- onboarding", "온보딩 화면 렌더", Some(ok("Command running in background with ID: b16yoo5kk.")), true),
        run("sleep 240 && ls shots/", "렌더 끝날 때까지 기다리기", Some(ToolOutcome { ok: false, exit: None, ms: Some(3), output: BLOCKED.into(), skipped: 0, diff: None }), false),
        Entry::Reply { text: "렌더를 배경에서 돌려 두었어요. 끝나면 바로 보여 드릴게요.".into() },
        Entry::User { text: DONE.into() },
        run("ls shots/onboarding", "렌더 결과 확인", Some(ok("1-welcome.png\n2-goals.png\n3-ready.png")), false),
        run("npm run contrast -- shots/onboarding", "대비 검사", Some(ok("Command running in background with ID: bq83mm1zx.")), true),
        Entry::User { text: FAILED.into() },
        Entry::User { text: "<system-reminder>\nThe task tools haven't been used recently.\n</system-reminder>".into() },
        Entry::Reply { text: "세 화면 렌더가 끝났어요. 대비 검사는 두 번째 화면의 보조 문구에서 걸렸어요 — 색을 한 단계 진하게 바꿔 다시 볼게요.".into() },
    ]
}
