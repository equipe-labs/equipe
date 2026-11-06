//! `FAKE_CHECKS=1`: the designer's conversation shows its method — a new
//! screen drawn and checked in code (`equipe design lint --render`), the floor
//! failing and fixed, a habit kept with its reason, then fresh eyes on it
//! (`equipe design second-look`): its first screenful alone, then the whole,
//! a fix list answered, and the hand-in. Every card is read from the lines
//! the real commands end with, made here by the same code.
//! `FAKE_CHECKS=fail`: the conversation stops at a check that fails the floor.

use equipe_core::design::lint::record::{Fix, Verdict};
use equipe_core::design::lint::{self, Summary};
use equipe_core::design::second_look;
use equipe_core::protocol::{Entry, ToolOutcome};

pub fn on() -> bool {
    std::env::var("FAKE_CHECKS").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn failing_only() -> bool {
    std::env::var("FAKE_CHECKS").is_ok_and(|v| v == "fail")
}

const SCREEN: &str = "settings/notifications";
const WIDTHS: [u32; 7] = [360, 390, 768, 1024, 1280, 1440, 1920];

fn pair(id: &str, said: &str) -> (String, String) {
    (id.to_owned(), said.to_owned())
}

fn first_check() -> Summary {
    Summary {
        floor: (9, 11),
        habits: 1,
        finish: 1,
        widths: WIDTHS.to_vec(),
        failing: vec![pair("contrast", "p.hint “Sent every day at 9 am” is 3.1:1 (#8A8C93 on #F6F5F1), under 4.5:1 (at 360, 390, 768, 1024, 1280, 1440, 1920 px)"), pair("token-space", "padding: 13px is off the spacing scale (4, 8, 12, 16, 24, 32)")],
        open: vec![pair("eyebrow", "2 of 2 heading(s) have a small label over them: “EMAIL”, “PUSH”")],
        notes: vec![pair("touch-target", "input#digest is 20×20")],
        screen: Some(SCREEN.into()),
        ..Summary::default()
    }
}

fn second_check() -> Summary {
    Summary {
        floor: (11, 11),
        habits: 0,
        waived: vec![pair("tracked-caps", "Locked in brand.md: channel names stay uppercase labels")],
        finish: 2,
        widths: WIDTHS.to_vec(),
        notes: vec![pair("touch-target", "input#digest is 20×20"), pair("literal-on-scale", "gap: 16px is `space.4`")],
        screen: Some(SCREEN.into()),
        ..Summary::default()
    }
}

fn lint_run(summary: &Summary, report: &str) -> Entry {
    let output = format!("Drawing design/mockups/notifications.html at 360,390,768,1024,1280,1440,1920…\n{report}\n{}", lint::mark_line(summary));
    let passed = summary.passes();
    Entry::Tool {
        tool: "Bash".into(),
        target: Some(format!("equipe design lint design/mockups/notifications.html --render --screen {SCREEN}")),
        outcome: Some(Box::new(ToolOutcome { ok: passed, exit: Some(if passed { 0 } else { 1 }), ms: Some(14_200), output, skipped: 0, diff: None })),
        label: Some("Screen check (7 widths)".into()),
        background: false,
    }
}

fn look_run(first: bool, verdict: &Verdict) -> Entry {
    let output = format!("A fresh Claude Code is looking at 7 picture(s) of {SCREEN}…\n{}{}", second_look::text(verdict), second_look::mark_line(SCREEN, verdict));
    Entry::Tool {
        tool: "Bash".into(),
        target: Some(format!("equipe design second-look {SCREEN}{} --criteria \"The digest turns off in one go\"", if first { " --first" } else { "" })),
        outcome: Some(Box::new(ToolOutcome { ok: verdict.pass, exit: Some(if verdict.pass { 0 } else { 1 }), ms: Some(48_000), output, skipped: 0, diff: None })),
        label: Some(if first { "Fresh eyes on the first screen" } else { "Fresh-eyes review" }.into()),
        background: false,
    }
}

fn edit(file: &str) -> Entry {
    Entry::Tool { tool: "Edit".into(), target: Some(format!("/tmp/fake/app/design/mockups/{file}")), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(20), ..ToolOutcome::default() })), label: None, background: false }
}

pub fn history() -> Vec<Entry> {
    let read = |f: &str| Entry::Tool { tool: "Read".into(), target: Some(format!("/tmp/fake/app/design/{f}")), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(30), ..ToolOutcome::default() })), label: None, background: false };
    let mut out = vec![
        Entry::Note { text: "── T-61 · Notification settings screen draft — on branch equipe/t-61".into() },
        Entry::User { text: "Make a draft of the notification settings screen. The digest has to turn off in one go.".into() },
        Entry::Reply { text: "I'll read the brief and the design system first, then make the first screen and check it.".into() },
        read("brand.md"),
        read("guidelines.md"),
        read("tokens.json"),
        Entry::Tool { tool: "Write".into(), target: Some("/tmp/fake/app/design/mockups/notifications.html".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(40), ..ToolOutcome::default() })), label: None, background: false },
        lint_run(&first_check(), "Floor — never waived; fix before handing in:\n  ✗ contrast …\n  ✗ token-space …"),
    ];
    if failing_only() {
        return out;
    }
    let first_look = Verdict { pass: true, summary: "The first screen opens on the digest switch, so what to do is plain at once. Build the rest on it.".into(), by: "claude".into(), widths: WIDTHS.to_vec(), first: true, ..Verdict::default() };
    let fixes = Verdict {
        pass: false,
        summary: "Turning it off in one go works, but at narrow widths you can't see that it saved.".into(),
        fixes: vec![
            Fix { place: "Digest row, 360·390".into(), what: "The “Saved” mark beside the switch wraps out of sight — move it under the switch".into(), why: "You can't be sure it's off".into() },
            Fix { place: "Push card".into(), what: "Set a turned-off channel's description in muted".into(), why: String::new() },
        ],
        by: "claude".into(),
        widths: WIDTHS.to_vec(),
        ..Verdict::default()
    };
    let passed = Verdict { pass: true, summary: "The digest turns off in one go, and the new state shows at every width.".into(), by: "claude".into(), widths: WIDTHS.to_vec(), ..Verdict::default() };
    out.extend([
        Entry::Reply { text: "Two baseline checks failed — the secondary text's contrast and one spacing value. I'll fix them, and keep the label above the title, an uppercase label locked in brand.md, with the reason noted.".into() },
        edit("notifications.html"),
        Entry::Tool {
            tool: "Bash".into(),
            target: Some("equipe design lint --waive tracked-caps --why \"Locked in brand.md: channel names stay uppercase labels\" --ticket T-61".into()),
            outcome: Some(Box::new(ToolOutcome { ok: true, exit: Some(0), ms: Some(60), output: "wrote design/decisions/2026-11-05-keep-tracked-caps.md — `tracked-caps` is kept with its reason.".into(), skipped: 0, diff: None })),
            label: Some("Note why the habit stays".into()),
            background: false,
        },
        lint_run(&second_check(), "Finish — worth doing, never blocking:\n  · touch-target …"),
        look_run(true, &first_look),
        look_run(false, &fixes),
        Entry::Reply { text: "Fresh eyes caught two things. I'll move the saved mark under the switch at narrow widths and fix the colour of a turned-off channel's description.".into() },
        edit("notifications.html"),
        lint_run(&second_check(), "Finish — worth doing, never blocking:\n  · touch-target …"),
        look_run(false, &passed),
        Entry::Tool { tool: "mcp__equipe__submit_for_review".into(), target: Some("Notification settings screen draft".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(80), ..ToolOutcome::default() })), label: None, background: false },
        Entry::Reply { text: "Handed in the draft. Baseline 11/11; the uppercase label stays, as brand.md locks it; the two things fresh eyes caught are fixed and pass.".into() },
    ]);
    out
}
